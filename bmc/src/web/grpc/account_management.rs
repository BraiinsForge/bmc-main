// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::collections::HashMap;
use std::panic;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bmc_field_schema::ParamKey;
use bmc_grpc::web;
use indexmap::IndexMap;
use prost_types::Timestamp;
use reqwest::StatusCode;
use thiserror::Error;
use tokio::sync::RwLock;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};
use tracing::{error, warn};

use crate::credential;
use crate::data::{Account, AccountId};
use crate::secret_store::SecretStoreHandle;
use crate::web::grpc::GrpcError;
use crate::web::grpc::shared::{FieldViolations, ParseOutput, unchecked_field_violations_status};

const POOL_API_URL: &str = "https://api.braiins.com/pool/v2";
const POOL_API_ENDPOINT: &str = "/user/hashrate/current";
const API_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct AccountManagementService {
    secret_store: Arc<RwLock<SecretStoreHandle>>,
}

impl AccountManagementService {
    pub(crate) fn new(secret_store: Arc<RwLock<SecretStoreHandle>>) -> Self {
        Self { secret_store }
    }
}

#[async_trait::async_trait]
impl web::account_management_service_server::AccountManagementService for AccountManagementService {
    async fn get_all_accounts(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetAllAccountsResponse>, Status> {
        let store = self.secret_store.read().await;
        let accounts = store
            .accounts()
            .values()
            .cloned()
            .map(map_account_to_proto)
            .collect();
        Ok(Response::new(web::GetAllAccountsResponse { accounts }))
    }

    async fn upsert_account(
        &self,
        request: Request<web::UpsertAccountRequest>,
    ) -> Result<Response<String>, Status> {
        let req = request.into_inner();

        let mut violations = FieldViolations::new();
        let (name, v) = parse_account_name("name", &req.name);
        violations.extend(v);
        let target_id = if req.id.is_empty() {
            None
        } else {
            let (id, v) = parse_account_id("id", &req.id);
            violations.extend(v);
            id
        };
        let field_values = parse_field_values("field_values", &req.field_values, &mut violations);
        if !violations.is_empty() {
            return Err(bad_request(violations));
        }
        let name = name.ok_or_else(unchecked_field_violations_status)?;

        // The type is fixed for an account's life: on create it comes from the request, on update
        // from the stored account (the request's type_id is ignored).
        let type_id = match &target_id {
            Some(id) => self
                .secret_store
                .read()
                .await
                .accounts()
                .get(id)
                .ok_or_else(|| Status::not_found("Account not found"))?
                .type_id
                .clone(),
            None => req.type_id.clone(),
        };

        // On update, an empty field_values map keeps the stored secrets — skip validation then.
        let replace_values = target_id.is_none() || !field_values.is_empty();
        if replace_values {
            validate_account(&type_id, &field_values).await?;
        }

        let join_handle = tokio::spawn({
            let secret_store = self.secret_store.clone();
            async move {
                let mut store = secret_store.write().await;
                let mut temp = store.clone();

                let id = if let Some(id) = target_id {
                    let account = temp
                        .accounts_mut()
                        .get_mut(&id)
                        .ok_or_else(|| Status::not_found("Account not found"))?;
                    account.name = name;
                    if replace_values {
                        account.field_values = field_values;
                    }
                    id
                } else {
                    let account = Account::new(type_id, name, field_values);
                    let id = account.id.clone();
                    temp.accounts_mut().insert(id.clone(), account);
                    id
                };

                if let Err(err) = temp.save().await {
                    error!("Cannot save accounts: {err}");
                    return Err(Status::internal("Failed to save accounts"));
                }
                *store = temp;
                Ok(Response::new(id.to_string()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn remove_account(&self, request: Request<String>) -> Result<Response<()>, Status> {
        let (id, violations) = parse_account_id("id", &request.into_inner());
        if !violations.is_empty() {
            return Err(bad_request(violations));
        }
        let id = id.ok_or_else(unchecked_field_violations_status)?;

        let join_handle = tokio::spawn({
            let secret_store = self.secret_store.clone();
            async move {
                let mut store = secret_store.write().await;
                let mut temp = store.clone();
                temp.accounts_mut()
                    .shift_remove(&id)
                    .ok_or_else(|| Status::not_found("Account not found"))?;
                if let Err(err) = temp.save().await {
                    error!("Cannot save accounts: {err}");
                    return Err(Status::internal("Failed to save accounts"));
                }
                *store = temp;
                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }
}

/// Field-violation key for a credential field (e.g. `token` → `field_values.token`), so the web UI
/// can attach the error to that field's input rather than showing it form-wide.
// Bracketed like every other map entry, so the key reaches the form unrenamed.
fn field_error_key(field: &str) -> String {
    format!("field_values[{field:?}]")
}

/// Check the field values against the type's schema, then run the per-type upstream check,
/// collecting every field violation and returning them together.
async fn validate_account(
    type_id: &str,
    field_values: &IndexMap<ParamKey, String>,
) -> Result<(), Status> {
    let mut violations = FieldViolations::new();
    validate_schema(type_id, field_values, &mut violations);
    // Only spend a network round-trip once the field shape is valid.
    if violations.is_empty() {
        validate_upstream(type_id, field_values, &mut violations).await;
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(bad_request(violations))
    }
}

/// `type_id` must name a known credential type, and `field_values` must supply exactly its fields,
/// each non-empty. Pure (no upstream call), so unit-testable on its own.
fn validate_schema(
    type_id: &str,
    field_values: &IndexMap<ParamKey, String>,
    violations: &mut FieldViolations,
) {
    let Some(cred_type) = credential::builtins().into_iter().find(|t| t.id == type_id) else {
        violations.push("type_id", format!("Unknown credential type {type_id:?}"));
        return;
    };

    for key in cred_type.fields.keys() {
        let has_value = field_values.get(key).is_some_and(|value| !value.is_empty());
        if !has_value {
            violations.push(field_error_key(key.as_str()), "Required");
        }
    }
    for key in field_values.keys() {
        if !cred_type.fields.contains_key(key) {
            violations.push(field_error_key(key.as_str()), "Unknown field");
        }
    }
}

/// Per-type live check. Braiins Pool verifies the token against the pool API; other types have no
/// upstream to check against and pass.
async fn validate_upstream(
    type_id: &str,
    field_values: &IndexMap<ParamKey, String>,
    violations: &mut FieldViolations,
) {
    if type_id != "braiins-pool" {
        return;
    }
    let token = field_values.get("token").map_or("", String::as_str);
    if let Err(err) = check_braiins_pool_token(token).await {
        violations.push(field_error_key("token"), err.to_string());
    }
}

async fn check_braiins_pool_token(token: &str) -> Result<(), CredentialCheckError> {
    let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
        warn!("Cannot build reqwest client");
        return Err(CredentialCheckError::ClientInitFailed);
    };

    let endpoint = format!("{POOL_API_URL}{POOL_API_ENDPOINT}");
    let status = match client.get(endpoint).header("X-API-Key", token).send().await {
        Ok(response) => response.status(),
        Err(err) => {
            warn!("Failed to reach pool API: {err}");
            return Err(CredentialCheckError::ServiceNotAvailable);
        }
    };

    match status {
        StatusCode::OK => Ok(()),
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => Err(CredentialCheckError::Invalid),
        status if status.is_server_error() => {
            warn!("Pool API server error: {status}");
            Err(CredentialCheckError::ServiceNotAvailable)
        }
        status => {
            warn!("Unexpected pool API status: {status}");
            Err(CredentialCheckError::Unexpected)
        }
    }
}

fn map_account_to_proto(account: Account) -> web::Account {
    web::Account {
        id: account.id.to_string(),
        type_id: account.type_id,
        name: account.name,
        created_at: Some(Timestamp {
            seconds: account.created_at.timestamp(),
            nanos: 0,
        }),
        connected_widgets: Vec::new(),
    }
}

fn parse_account_name(field: &str, input: &str) -> ParseOutput<String> {
    let mut violations = FieldViolations::new();
    let maybe_name = match input.len() {
        0 => {
            violations.push(field, "Missing value!");
            None
        }
        1..=50 => Some(input.to_owned()),
        _ => {
            violations.push(field, "Input too long!");
            None
        }
    };
    (maybe_name, violations)
}

fn parse_account_id(field: &str, input: &str) -> ParseOutput<AccountId> {
    let mut violations = FieldViolations::new();
    let maybe_id = AccountId::from_str(input).ok();
    if maybe_id.is_none() {
        violations.push(field, "Invalid account ID");
    }
    (maybe_id, violations)
}

fn parse_field_values(
    field: &str,
    input: &HashMap<String, String>,
    violations: &mut FieldViolations,
) -> IndexMap<ParamKey, String> {
    let mut out = IndexMap::with_capacity(input.len());
    for (key, value) in input {
        match ParamKey::try_new(key.clone()) {
            Ok(key) => {
                out.insert(key, value.clone());
            }
            Err(bad) => violations.push(field, format!("Invalid field key {bad:?}")),
        }
    }
    out
}

fn bad_request(violations: FieldViolations) -> Status {
    Status::with_error_details(
        Code::InvalidArgument,
        GrpcError::BadRequest.to_string(),
        ErrorDetails::with_bad_request(violations),
    )
}

#[derive(Debug, Clone, Error)]
enum CredentialCheckError {
    #[error("Service or internet unavailable")]
    ServiceNotAvailable,
    #[error("Invalid API key")]
    Invalid,
    #[error("Unexpected response from the credential service")]
    Unexpected,
    #[error("Failed to initialize HTTP client")]
    ClientInitFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic_types::FieldViolation;

    fn values(pairs: &[(&str, &str)]) -> IndexMap<ParamKey, String> {
        pairs
            .iter()
            .map(|(key, value)| {
                let key = ParamKey::try_new((*key).to_owned()).expect("BUG: test key is valid");
                (key, (*value).to_owned())
            })
            .collect()
    }

    /// Collect the field violations `validate_schema` produces for a (type, fields) pair.
    fn schema_violations(type_id: &str, pairs: &[(&str, &str)]) -> Vec<FieldViolation> {
        let mut violations = FieldViolations::new();
        validate_schema(type_id, &values(pairs), &mut violations);
        violations.into()
    }

    #[test]
    fn schema_accepts_matching_values() {
        assert!(schema_violations("generic-token", &[("token", "abc")]).is_empty());
        assert!(
            schema_violations("generic-userpass", &[("username", "u"), ("password", "p")])
                .is_empty()
        );
    }

    #[test]
    fn schema_rejects_unknown_type() {
        assert_eq!(schema_violations("nope", &[])[0].field, "type_id");
    }

    #[test]
    fn schema_keys_each_missing_field_to_its_own_path() {
        // Both fields missing → one violation each, keyed under the field's own path so the web UI
        // can attach them to the right inputs.
        let violations = schema_violations("generic-userpass", &[]);
        let fields: Vec<&str> = violations.iter().map(|v| v.field.as_str()).collect();
        assert_eq!(
            fields,
            [r#"field_values["username"]"#, r#"field_values["password"]"#]
        );
    }

    #[test]
    fn schema_flags_empty_and_unknown_field() {
        assert_eq!(
            schema_violations("generic-token", &[("token", "")])[0].field,
            r#"field_values["token"]"#,
        );
        assert_eq!(
            schema_violations("generic-token", &[("token", "abc"), ("extra", "x")])[0].field,
            r#"field_values["extra"]"#,
        );
    }
}

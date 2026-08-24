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

use std::collections::{HashMap, HashSet};
use std::panic;
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use bmc_field_schema::ParamKey;
use bmc_field_schema::credential::BRAIINS_POOL_HOST;
use bmc_grpc::web;
use indexmap::IndexMap;
use prost_types::Timestamp;
use reqwest::StatusCode;
use thiserror::Error;
use tokio::sync::RwLock;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};
use tracing::{error, warn};
use url::Url;

use crate::config::ConfigHandle;
use crate::credential;
use crate::data::{Account, AccountId};
use crate::scene::{Scene, SceneId};
use crate::secret_store::SecretStoreHandle;
use crate::web::grpc::GrpcError;
use crate::web::grpc::shared::{FieldViolations, ParseOutput, unchecked_field_violations_status};

/// Where the Braiins Pool token check goes.
static POOL_ENDPOINT: LazyLock<Url> = LazyLock::new(|| {
    Url::parse(&format!(
        "https://{BRAIINS_POOL_HOST}/pool/v2/user/hashrate/current"
    ))
    .expect("BUG: the pinned host must form a valid endpoint URL")
});
const API_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct AccountManagementService {
    /// Lock order wherever both are held: this first, then `secret_store`.
    config_handle: Arc<RwLock<ConfigHandle>>,
    secret_store: Arc<RwLock<SecretStoreHandle>>,
}

impl AccountManagementService {
    pub(crate) fn new(
        config_handle: Arc<RwLock<ConfigHandle>>,
        secret_store: Arc<RwLock<SecretStoreHandle>>,
    ) -> Self {
        Self {
            config_handle,
            secret_store,
        }
    }
}

#[async_trait::async_trait]
impl web::account_management_service_server::AccountManagementService for AccountManagementService {
    async fn get_all_accounts(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetAllAccountsResponse>, Status> {
        let mut bound = bindings_by_account(self.config_handle.read().await.scenes());
        let store = self.secret_store.read().await;
        let accounts = store
            .accounts()
            .values()
            .cloned()
            .map(|account| {
                let widgets = bound.remove(&account.id).unwrap_or_default();
                map_account_to_proto(account, widgets)
            })
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

        let allow_hosts = req.allow_hosts;
        {
            let mut violations = FieldViolations::new();
            validate_allow_hosts(&type_id, &allow_hosts, &mut violations);
            if !violations.is_empty() {
                return Err(bad_request(violations));
            }
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
                    let effective_values = if replace_values {
                        field_values
                    } else {
                        account.field_values.clone()
                    };
                    if account.name == name
                        && account.field_values == effective_values
                        && account.allow_hosts == allow_hosts
                    {
                        return Ok(Response::new(id.to_string()));
                    }
                    account.name = name;
                    account.field_values = effective_values;
                    account.allow_hosts = allow_hosts;
                    id
                } else {
                    let mut account = Account::new(type_id, name, field_values);
                    account.allow_hosts = allow_hosts;
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

        // Unbind and delete under one config write lock,
        // so a binding written in between cannot outlive the account it names.
        // Both locks sit inside the task: a dropped `JoinHandle` does not cancel it,
        // so a guard held out here would fall to a client hanging up mid-delete.
        //
        // Config before secrets, per the lock order.
        // A failed delete leaves the account merely unbound, which a retry finishes;
        // the reverse order would leave bindings pointing at an account that is gone.
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let secret_store = self.secret_store.clone();
            async move {
                let mut config = config_handle.write().await;
                let mut store = secret_store.write().await;
                if !store.accounts().contains_key(&id) {
                    return Err(Status::not_found("Account not found"));
                }

                let mut new_config = config.clone();
                if unbind_account(new_config.scenes_mut(), &id) {
                    new_config
                        .save()
                        .await
                        .map_err(|e| Status::internal(format!("failed to save config: {e}")))?;
                    *config = new_config;
                }

                let mut temp = store.clone();
                temp.accounts_mut()
                    .shift_remove(&id)
                    .ok_or_else(|| Status::not_found("Account not found"))?;
                let delete_error = match temp.save().await {
                    Ok(()) => {
                        *store = temp;
                        None
                    }
                    Err(err) => {
                        error!("Cannot save accounts: {err}");
                        Some(Status::internal("Failed to save accounts"))
                    }
                };
                if let Some(error) = delete_error {
                    return Err(error);
                }
                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }
}

/// Field-violation key for a credential field (e.g. `token` → `field_values.token`),
/// so the web UI can attach the error to that field rather than showing it form-wide.
// Bracketed like every other map entry, so the key reaches the form unrenamed.
fn field_error_key(field: &str) -> String {
    format!("field_values[{field:?}]")
}

/// Check the field values against the type's schema,
/// then run the per-type upstream check, collecting
/// every field violation and returning them together.
async fn validate_account(
    type_id: &str,
    field_values: &IndexMap<ParamKey, String>,
) -> Result<(), Status> {
    let mut violations = FieldViolations::new();
    validate_schema(type_id, field_values, &mut violations);
    if !violations.is_empty() {
        return Err(bad_request(violations));
    }
    // Only spend a network round-trip once the field shape is valid.
    validate_upstream(type_id, field_values).await
}

/// `type_id` must name a known credential type, and `field_values`
/// must supply exactly its fields, each non-empty.
/// Pure (no upstream call), so unit-testable on its own.
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
) -> Result<(), Status> {
    if type_id != credential::BuiltinType::BraiinsPool.id() {
        return Ok(());
    }
    let token = field_values.get("token").map_or("", String::as_str);
    match check_braiins_pool_token(token).await {
        Ok(()) => Ok(()),
        Err(err) => Err(upstream_check_status(&err)),
    }
}

/// A rejected token is the operator's to correct, so it stays an invalid-argument. Any
/// other outcome means the check could not run, and calling that invalid would send the
/// operator chasing a value that may be fine. Both keep the violation on the token field.
fn upstream_check_status(err: &CredentialCheckError) -> Status {
    let mut violations = FieldViolations::new();
    violations.push(field_error_key("token"), err.to_string());

    match err {
        CredentialCheckError::Invalid => bad_request(violations),
        CredentialCheckError::ServiceNotAvailable
        | CredentialCheckError::Unexpected
        | CredentialCheckError::ClientInitFailed => Status::with_error_details(
            Code::Unavailable,
            GrpcError::CredentialUnverified.to_string(),
            ErrorDetails::with_bad_request(violations),
        ),
    }
}

async fn check_braiins_pool_token(token: &str) -> Result<(), CredentialCheckError> {
    let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
        warn!("Cannot build reqwest client");
        return Err(CredentialCheckError::ClientInitFailed);
    };

    let status = match client
        .get(POOL_ENDPOINT.clone())
        .header("X-API-Key", token)
        .send()
        .await
    {
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

/// Widget instances bound to each account.
/// A widget that spends the same account on two slots
/// is still one widget, so its id appears once.
fn bindings_by_account(scenes: &IndexMap<SceneId, Scene>) -> HashMap<AccountId, Vec<String>> {
    let mut bound: HashMap<AccountId, Vec<String>> = HashMap::new();
    for scene in scenes.values() {
        for widget in scene.widgets.values() {
            let mut seen = HashSet::new();
            for account_id in widget.credential_bindings.values() {
                if seen.insert(account_id) {
                    bound
                        .entry(account_id.clone())
                        .or_default()
                        .push(widget.id.to_string());
                }
            }
        }
    }
    bound
}

/// Drop every binding naming `id`, reporting whether anything changed
/// so an unaffected config is not rewritten.
fn unbind_account(scenes: &mut IndexMap<SceneId, Scene>, id: &AccountId) -> bool {
    let mut unbound = false;
    for scene in scenes.values_mut() {
        for widget in scene.widgets.values_mut() {
            let before = widget.credential_bindings.len();
            widget.credential_bindings.retain(|_, bound| bound != id);
            unbound |= widget.credential_bindings.len() != before;
        }
    }
    unbound
}

fn map_account_to_proto(account: Account, connected_widgets: Vec<String>) -> web::Account {
    web::Account {
        id: account.id.to_string(),
        type_id: account.type_id,
        name: account.name,
        created_at: Some(Timestamp {
            seconds: account.created_at.timestamp(),
            nanos: 0,
        }),
        connected_widgets,
        allow_hosts: account.allow_hosts,
    }
}

/// Reject a list on a pinned type, and any entry the pin grammar cannot use.
/// Runs against the request even though a hand-edited store is honoured as-is:
/// the API is the form's contract, and the form must learn of a dead line now
/// rather than ship it.
fn validate_allow_hosts(type_id: &str, entries: &[String], violations: &mut FieldViolations) {
    const FIELD: &str = "allow_hosts";

    if entries.is_empty() {
        return;
    }
    let pinned = credential::BuiltinType::from_id(type_id)
        .and_then(credential::BuiltinType::egress)
        .is_some();
    if pinned {
        violations.push(
            FIELD.to_owned(),
            "This credential type already pins where its secret may go",
        );
        return;
    }
    for (index, entry) in entries.iter().enumerate() {
        if let Err(reason) = credential::check_entry(entry) {
            violations.push(FIELD.to_owned(), format!("Line {}: {reason}", index + 1));
        }
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
    #[error("Could not reach Braiins Pool, so this token is unverified")]
    ServiceNotAvailable,
    #[error("Invalid API key")]
    Invalid,
    #[error("Braiins Pool gave an unexpected response, so this token is unverified")]
    Unexpected,
    #[error("Could not run the check, so this token is unverified")]
    ClientInitFailed,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    use proptest::prelude::*;
    use tonic_types::FieldViolation;

    use super::*;
    use crate::compositor::testing::RecordingCompositor;
    use crate::compositor::{
        Compositor, CredentialSecrets, DisplayInfo, DisplayShape, HardwareCapabilities, SlotGrid,
        WidgetConnectionMode, WidgetInitialConfig, WidgetInstanceKey, WidgetRegistration,
    };
    use crate::scene::{SceneKind, Widget, WidgetPlacement, WidgetPosition};
    use crate::widget::{Coordinator, WidgetManager};

    const TEST_WIDGET_TYPE: &str = "550e8400-e29b-41d4-a716-446655440000";

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

    fn allow_hosts_violations(type_id: &str, entries: &[&str]) -> Vec<FieldViolation> {
        let entries: Vec<String> = entries.iter().map(|e| (*e).to_owned()).collect();
        let mut violations = FieldViolations::new();
        validate_allow_hosts(type_id, &entries, &mut violations);
        violations.into()
    }

    #[test]
    fn a_host_list_on_a_pinned_type_is_refused() {
        let violations = allow_hosts_violations(
            credential::BuiltinType::BraiinsPool.id(),
            &["api.example.com"],
        );

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, "allow_hosts");
    }

    #[test]
    fn a_bad_line_is_named_by_its_position() {
        let violations = allow_hosts_violations(
            credential::BuiltinType::GenericToken.id(),
            &["api.example.com", "not a host"],
        );

        assert_eq!(violations.len(), 1);
        assert!(
            violations[0].description.starts_with("Line 2:"),
            "the form has one textarea, so the message must point into it: {}",
            violations[0].description
        );
    }

    #[test]
    fn clearing_the_list_stays_legal_on_a_pinned_type() {
        // The refusal is of *setting* a list, not of the field existing.
        // An account that once carried one must still be able to drop it,
        // and reordering the two guards would break that silently.
        assert!(allow_hosts_violations(credential::BuiltinType::BraiinsPool.id(), &[]).is_empty());
    }

    #[test]
    fn every_bad_line_is_reported_not_just_the_first() {
        // One textarea, so the operator fixes what the list tells them.
        // Stopping at the first would make them submit once per mistake.
        let violations = allow_hosts_violations(
            credential::BuiltinType::GenericToken.id(),
            &["not a host", "api.example.com", "also bad"],
        );

        let lines: Vec<&str> = violations
            .iter()
            .map(|v| v.description.as_str())
            .filter_map(|d| d.split(':').next())
            .collect();
        assert_eq!(lines, ["Line 1", "Line 3"]);
    }

    #[test]
    fn a_clean_list_on_an_unpinned_type_passes() {
        assert!(
            allow_hosts_violations(
                credential::BuiltinType::GenericToken.id(),
                &["api.example.com", "*.example.org", "10.0.0.0/8"],
            )
            .is_empty()
        );
    }

    fn violation_fields(status: &Status) -> Vec<String> {
        status
            .get_error_details()
            .bad_request()
            .map(|bad| {
                bad.field_violations
                    .iter()
                    .map(|v| v.field.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn schema_accepts_matching_values() {
        assert!(
            schema_violations(
                credential::BuiltinType::GenericToken.id(),
                &[("token", "abc")]
            )
            .is_empty()
        );
        assert!(
            schema_violations(
                credential::BuiltinType::GenericUserpass.id(),
                &[("username", "u"), ("password", "p")]
            )
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
        let violations = schema_violations(credential::BuiltinType::GenericUserpass.id(), &[]);
        let fields: Vec<&str> = violations.iter().map(|v| v.field.as_str()).collect();
        assert_eq!(
            fields,
            [r#"field_values["username"]"#, r#"field_values["password"]"#]
        );
    }

    #[test]
    fn schema_flags_empty_and_unknown_field() {
        assert_eq!(
            schema_violations(credential::BuiltinType::GenericToken.id(), &[("token", "")])[0]
                .field,
            r#"field_values["token"]"#,
        );
        assert_eq!(
            schema_violations(
                credential::BuiltinType::GenericToken.id(),
                &[("token", "abc"), ("extra", "x")]
            )[0]
            .field,
            r#"field_values["extra"]"#,
        );
    }

    /// A one-widget scene whose slots bind the given accounts, keyed `slot0`, `slot1`, …
    /// A widget whose slots, keyed `slot0`, `slot1`, …, bind the given accounts.
    fn widget_binding(accounts: &[&str]) -> Widget {
        let mut widget = Widget::new(
            uuid::Uuid::parse_str(TEST_WIDGET_TYPE).expect("BUG: test widget type is valid"),
            BTreeMap::new(),
            WidgetPosition { row: 0, col: 0 },
            WidgetPlacement::Fullscreen,
        );
        for (i, account) in accounts.iter().enumerate() {
            let slot = serde_json::from_str(&format!("\"slot{i}\"")).expect("BUG: valid slot key");
            let id = AccountId::from_str(account).expect("BUG: non-empty id");
            widget.credential_bindings.insert(slot, id);
        }
        widget
    }

    fn scene_of(kind: SceneKind, widgets: Vec<Widget>) -> Scene {
        Scene {
            id: SceneId::generate(),
            enabled: true,
            cycle_duration: None,
            kind,
            widgets: widgets.into_iter().map(|w| (w.id, w)).collect(),
        }
    }

    fn scene_binding(accounts: &[&str]) -> (Scene, String) {
        let widget = widget_binding(accounts);
        let widget_id = widget.id.to_string();

        (scene_of(SceneKind::Fullscreen, vec![widget]), widget_id)
    }

    /// A combined scene holding one widget per entry, with that entry's bindings.
    fn combined_scene(widgets: &[&[&str]]) -> (Scene, Vec<String>) {
        let widgets: Vec<Widget> = widgets.iter().copied().map(widget_binding).collect();
        let ids = widgets.iter().map(|w| w.id.to_string()).collect();

        (scene_of(SceneKind::Combined, widgets), ids)
    }

    fn scenes_of(scenes: Vec<Scene>) -> IndexMap<SceneId, Scene> {
        scenes.into_iter().map(|scene| (scene.id, scene)).collect()
    }

    fn install_account_test_widget(tmp: &tempfile::TempDir) -> Vec<std::path::PathBuf> {
        let package = tmp.path().join("account-test-widget");
        std::fs::create_dir(&package).expect("BUG: create test widget package");
        std::fs::write(
            package.join("manifest.json"),
            format!(
                r#"{{"uid":"{TEST_WIDGET_TYPE}","version":"1.0.0","name":"account-test","description":"account test","binary":"widget","supported_viewports":[{{"type":"rectangular","min_width":1280,"max_width":1280,"min_height":480,"max_height":480}}],"credentials":{{"slot0":{{"type":"generic-token","label":"First"}},"slot1":{{"type":"generic-token","label":"Second"}},"slot2":{{"type":"generic-token","label":"Third"}}}}}}"#
            ),
        )
        .expect("BUG: write test widget manifest");
        let binary = package.join("widget");
        std::fs::write(&binary, "#!/bin/sh\nexit 0\n").expect("BUG: write test widget binary");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: make test widget executable");
        vec![tmp.path().to_path_buf()]
    }

    fn register_retained_widgets(compositor: &RecordingCompositor, widgets: &[Widget]) {
        for widget in widgets {
            compositor
                .enqueue_register_widget(WidgetRegistration {
                    key: widget.id.into(),
                    connection_mode: WidgetConnectionMode::Accepting,
                    initial_config: WidgetInitialConfig {
                        width: 1280,
                        height: 480,
                        viewport_shape: bmc_widget_protocol::ViewportShape::Rectangular,
                        display: bmc_widget_protocol::DisplayInfo::BMC100,
                        params: serde_json::Map::new(),
                        credentials: serde_json::Map::new(),
                        credential_secrets: CredentialSecrets::default(),
                        token: widget.id.to_string(),
                    },
                })
                .expect("BUG: seed retained widget");
        }
    }

    async fn prime_retained_credentials(
        coordinator: &Coordinator,
        compositor: &RecordingCompositor,
        widgets: &[Widget],
    ) {
        for widget in widgets {
            let resolved = coordinator
                .resolve_credentials(widget)
                .await
                .expect("BUG: seeded widget manifest is installed");
            compositor
                .enqueue_update_widget_credentials(
                    WidgetInstanceKey::new(widget.id.as_uuid()),
                    resolved.view,
                    resolved.secrets,
                )
                .expect("BUG: seed retained credentials")
                .wait()
                .await
                .expect("BUG: seed credential receipt");
        }
        compositor.clear_credential_pushes();
    }

    #[test]
    fn an_unbound_account_has_no_connected_widgets() {
        let (scene, _) = scene_binding(&[]);

        assert!(bindings_by_account(&scenes_of(vec![scene])).is_empty());
    }

    #[test]
    fn a_bound_account_names_the_widget_holding_it() {
        let (scene, widget_id) = scene_binding(&["acct-1"]);
        let bound = bindings_by_account(&scenes_of(vec![scene]));

        let id = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        assert_eq!(bound.get(&id), Some(&vec![widget_id]));
    }

    #[test]
    fn one_widget_spending_an_account_twice_still_counts_once() {
        let (scene, widget_id) = scene_binding(&["acct-1", "acct-1"]);
        let bound = bindings_by_account(&scenes_of(vec![scene]));

        let id = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        assert_eq!(bound.get(&id), Some(&vec![widget_id]));
    }

    #[test]
    fn a_scene_of_several_widgets_reports_each_one_that_binds_the_account() {
        let (scene, ids) = combined_scene(&[&["acct-1"], &[], &["acct-1", "acct-2"]]);
        let bound = bindings_by_account(&scenes_of(vec![scene]));

        let shared = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        assert_eq!(
            bound.get(&shared),
            Some(&vec![ids[0].clone(), ids[2].clone()]),
            "the unbound middle widget must not shift the reported set"
        );

        let single = AccountId::from_str("acct-2").expect("BUG: non-empty id");
        assert_eq!(bound.get(&single), Some(&vec![ids[2].clone()]));
    }

    #[test]
    fn an_account_bound_across_scenes_collects_every_widget() {
        let (first, first_id) = scene_binding(&["acct-1"]);
        let (second, second_id) = scene_binding(&["acct-1"]);
        let bound = bindings_by_account(&scenes_of(vec![first, second]));

        let id = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        assert_eq!(bound.get(&id), Some(&vec![first_id, second_id]));
    }

    /// A service over a real config and secret store, seeded with `scenes`
    /// and one `generic-token` account per id in `accounts`.
    async fn seeded_service(
        tmp: &tempfile::TempDir,
        scenes: Vec<Scene>,
        accounts: &[&str],
    ) -> (
        AccountManagementService,
        Arc<RwLock<ConfigHandle>>,
        Arc<RwLock<SecretStoreHandle>>,
        Arc<Coordinator>,
        Arc<RecordingCompositor>,
    ) {
        let config_path = tmp.path().join("bmc-config.json");
        let config_handle = Arc::new(RwLock::new(
            ConfigHandle::init(
                config_path.clone(),
                50,
                50,
                50,
                50,
                bmc_platform::Product::Bmc100,
            )
            .await
            .0,
        ));
        {
            let mut config = config_handle.write().await;
            config.scenes_mut().clear();
            for scene in scenes {
                config.scenes_mut().insert(scene.id, scene);
            }
            config.save().await.expect("BUG: seed config must save");
        }

        let mut store = SecretStoreHandle::init(&config_path).await;
        for raw in accounts {
            let id = AccountId::from_str(raw).expect("BUG: non-empty id");
            store.accounts_mut().insert(
                id.clone(),
                Account {
                    id,
                    type_id: credential::BuiltinType::GenericToken.id().to_owned(),
                    name: (*raw).to_owned(),
                    field_values: values(&[("token", "s3cr3t")]),
                    allow_hosts: Vec::new(),
                    created_at: chrono::Utc::now(),
                },
            );
        }
        store.save().await.expect("BUG: seed store must save");
        let secret_store = Arc::new(RwLock::new(store));

        let widgets = config_handle
            .read()
            .await
            .scenes()
            .values()
            .flat_map(|scene| scene.widgets.values().cloned())
            .collect::<Vec<_>>();
        let manager = WidgetManager::init(install_account_test_widget(tmp), false).await;
        let registry = manager.registry();
        let compositor = Arc::new(RecordingCompositor::default());
        register_retained_widgets(&compositor, &widgets);
        let widget_coordinator = Arc::new(Coordinator::new(
            manager,
            Arc::clone(&compositor) as Arc<dyn Compositor>,
            Some("test-display".to_owned()),
            registry,
            HardwareCapabilities {
                display: DisplayInfo {
                    width: 1280,
                    height: 480,
                    shape: DisplayShape::Rectangular,
                    dpi: 1,
                },
                slot_grid: Some(SlotGrid {
                    columns: 4,
                    rows: 2,
                }),
            },
            Arc::clone(&secret_store),
        ));
        prime_retained_credentials(&widget_coordinator, &compositor, &widgets).await;

        let scenes_rx = config_handle.read().await.subscribe_scenes_change();
        let accounts_rx = secret_store.read().await.subscribe_accounts_change();
        crate::widget::coordinator::start_credential_listener(
            Arc::clone(&widget_coordinator),
            Arc::clone(&config_handle),
            scenes_rx,
            accounts_rx,
        );

        let service =
            AccountManagementService::new(Arc::clone(&config_handle), Arc::clone(&secret_store));
        (
            service,
            config_handle,
            secret_store,
            widget_coordinator,
            compositor,
        )
    }

    fn pushed_widget_ids(compositor: &RecordingCompositor) -> Vec<String> {
        let mut ids = compositor
            .credential_pushes()
            .into_iter()
            .map(|push| push.instance_id)
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    fn pushed_widget_changes(compositor: &RecordingCompositor) -> Vec<(String, bool)> {
        let mut changes = compositor
            .credential_pushes()
            .into_iter()
            .map(|push| (push.instance_id, push.changed))
            .collect::<Vec<_>>();
        changes.sort();
        changes
    }

    fn sorted_widget_ids(widgets: &[&Widget]) -> Vec<String> {
        let mut ids = widgets
            .iter()
            .map(|widget| widget.id.to_string())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    fn sorted_widget_changes(widgets: &[(&Widget, bool)]) -> Vec<(String, bool)> {
        let mut changes = widgets
            .iter()
            .map(|(widget, changed)| (widget.id.to_string(), *changed))
            .collect::<Vec<_>>();
        changes.sort();
        changes
    }

    fn account_update_request(token: &str) -> Request<web::UpsertAccountRequest> {
        Request::new(web::UpsertAccountRequest {
            id: "acct-1".to_owned(),
            type_id: String::new(),
            name: "acct-1".to_owned(),
            field_values: HashMap::from([("token".to_owned(), token.to_owned())]),
            allow_hosts: Vec::new(),
        })
    }

    fn replace_file_with_directory(path: &std::path::Path) {
        std::fs::remove_file(path).expect("BUG: seeded persistence file must exist");
        std::fs::create_dir(path).expect("BUG: replace persistence file with a directory");
    }

    #[tokio::test]
    async fn account_changes_refresh_all_widgets_and_retry_only_changed_ones() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let first = widget_binding(&["acct-1", "acct-1"]);
        let second = widget_binding(&["acct-1"]);
        let other = widget_binding(&["acct-2"]);
        let expected_changes =
            sorted_widget_changes(&[(&first, true), (&second, true), (&other, false)]);
        let scene = scene_of(SceneKind::Combined, vec![first, second, other]);
        let (service, _config, _store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1", "acct-2"]).await;

        let request = |name: &str, token: &str, allow_hosts: &[&str]| {
            Request::new(web::UpsertAccountRequest {
                id: "acct-1".to_owned(),
                type_id: String::new(),
                name: name.to_owned(),
                field_values: HashMap::from([("token".to_owned(), token.to_owned())]),
                allow_hosts: allow_hosts.iter().map(|host| (*host).to_owned()).collect(),
            })
        };

        service
            .upsert_account(request("renamed", "s3cr3t", &[]))
            .await
            .expect("BUG: name update must succeed");
        compositor.wait_for_credential_push_count(3).await;
        assert_eq!(pushed_widget_changes(&compositor), expected_changes);
        compositor.clear_credential_pushes();

        service
            .upsert_account(request("renamed", "replacement", &[]))
            .await
            .expect("BUG: field update must succeed");
        compositor.wait_for_credential_push_count(3).await;
        assert_eq!(pushed_widget_changes(&compositor), expected_changes);
        compositor.clear_credential_pushes();

        service
            .upsert_account(request(
                "renamed",
                "replacement",
                &["b.example", "a.example"],
            ))
            .await
            .expect("BUG: host update must succeed");
        compositor.wait_for_credential_push_count(3).await;
        assert_eq!(pushed_widget_changes(&compositor), expected_changes);
    }

    #[tokio::test]
    async fn one_failed_credential_enqueue_does_not_skip_later_widgets() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let first = widget_binding(&["acct-1"]);
        let second = widget_binding(&["acct-1"]);
        let scene = scene_of(SceneKind::Combined, vec![first, second]);
        let (service, _config, _store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;
        compositor.fail_next_credential_update();

        service
            .upsert_account(account_update_request("replacement"))
            .await
            .expect("BUG: one widget enqueue failure must not fail the account update");

        compositor.wait_for_credential_push_count(1).await;
        assert_eq!(pushed_widget_ids(&compositor).len(), 1);
    }

    #[tokio::test]
    async fn missing_manifest_does_not_block_later_listener_refreshes() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let missing = widget_binding(&["acct-1"]);
        let installed = widget_binding(&["acct-1"]);
        let installed_id = installed.id.to_string();
        let scene = scene_of(SceneKind::Combined, vec![missing, installed]);
        let (service, config, _store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;
        config
            .write()
            .await
            .scenes_mut()
            .values_mut()
            .next()
            .expect("BUG: seeded scene")
            .widgets
            .first_mut()
            .expect("BUG: missing-manifest widget")
            .1
            .widget_type_id = uuid::Uuid::new_v4();

        service
            .upsert_account(account_update_request("replacement"))
            .await
            .expect("BUG: account update must succeed");

        compositor.wait_for_credential_push_count(1).await;
        assert_eq!(pushed_widget_ids(&compositor), vec![installed_id]);
    }

    #[tokio::test]
    async fn dropped_credential_receipts_are_awaited_without_source_locks() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let scene = scene_of(SceneKind::Fullscreen, vec![widget_binding(&["acct-1"])]);
        let (service, config, store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;
        compositor.hold_credential_receipts();
        let update = tokio::spawn(async move {
            service
                .upsert_account(account_update_request("replacement"))
                .await
        });
        compositor.wait_for_credential_push_count(1).await;

        let config_guard = tokio::time::timeout(Duration::from_secs(1), config.write())
            .await
            .expect("credential receipt wait must not hold the config lock");
        drop(config_guard);
        let store_guard = tokio::time::timeout(Duration::from_secs(1), store.write())
            .await
            .expect("credential receipt wait must not hold the secret-store lock");
        drop(store_guard);
        compositor.drop_credential_receipts();

        update
            .await
            .expect("BUG: account update task must not panic")
            .expect("BUG: a dropped widget receipt must not fail the account update");
    }

    #[tokio::test]
    async fn cancelled_account_rpc_finishes_enqueued_refreshes() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let scene = scene_of(SceneKind::Fullscreen, vec![widget_binding(&["acct-1"])]);
        let (service, _config, store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;
        compositor.hold_credential_receipts();
        let store_guard = store.write().await;
        let update = tokio::spawn(async move {
            service
                .upsert_account(Request::new(web::UpsertAccountRequest {
                    id: String::new(),
                    type_id: credential::BuiltinType::GenericToken.id().to_owned(),
                    name: "cancelled-create".to_owned(),
                    field_values: HashMap::from([("token".to_owned(), "replacement".to_owned())]),
                    allow_hosts: Vec::new(),
                }))
                .await
        });
        tokio::task::yield_now().await;

        update.abort();
        assert!(
            update
                .await
                .expect_err("account RPC must still be blocked on the secret store")
                .is_cancelled(),
            "account RPC must be cancelled while its detached update is pending"
        );
        drop(store_guard);
        compositor.wait_for_credential_push_count(1).await;
        compositor.release_credential_receipts();
        store
            .write()
            .await
            .save()
            .await
            .expect("BUG: listener barrier save must succeed");
        compositor.wait_for_credential_push_count(2).await;

        assert!(
            store
                .read()
                .await
                .accounts()
                .values()
                .any(|account| account.name == "cancelled-create"),
            "the detached account update must survive RPC cancellation"
        );
    }

    #[tokio::test]
    async fn account_deletion_holds_config_while_waiting_for_secrets() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let scene = scene_of(SceneKind::Fullscreen, vec![widget_binding(&["acct-1"])]);
        let (service, config, store, _coordinator, _compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;
        let secret_guard = store.write().await;
        let deletion = tokio::spawn(async move {
            service
                .remove_account(Request::new("acct-1".to_owned()))
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if config.try_read().is_err() {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("account deletion must acquire config before waiting for secrets");

        assert!(
            tokio::time::timeout(Duration::from_millis(20), config.write())
                .await
                .is_err(),
            "a competing config writer must serialize behind account deletion"
        );
        drop(secret_guard);
        deletion
            .await
            .expect("BUG: account deletion task must not panic")
            .expect("BUG: account deletion must converge after secrets are released");
        let _config_guard = tokio::time::timeout(Duration::from_secs(1), config.write())
            .await
            .expect("config lock must be released after account deletion");
    }

    #[tokio::test]
    async fn account_update_does_not_hold_config_while_waiting_for_secrets() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let scene = scene_of(SceneKind::Fullscreen, vec![widget_binding(&["acct-1"])]);
        let (service, config, store, _coordinator, _compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;
        let secret_guard = store.read().await;
        let update = tokio::spawn(async move {
            service
                .upsert_account(account_update_request("replacement"))
                .await
        });
        let config_guard = tokio::time::timeout(Duration::from_secs(1), config.write())
            .await
            .expect("an account-only update must not acquire the config lock");
        drop(config_guard);
        drop(secret_guard);
        update
            .await
            .expect("BUG: account update task must not panic")
            .expect("BUG: account update must converge after secrets are released");
        let _config_guard = tokio::time::timeout(Duration::from_secs(1), config.write())
            .await
            .expect("config lock must be released after account update");
    }

    #[tokio::test]
    async fn account_no_ops_do_not_wake_but_creation_is_a_semantic_noop() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let scene = scene_of(SceneKind::Fullscreen, vec![widget_binding(&["acct-1"])]);
        let (service, _config, _store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;
        let request = |id: &str, fields: HashMap<String, String>| {
            Request::new(web::UpsertAccountRequest {
                id: id.to_owned(),
                type_id: credential::BuiltinType::GenericToken.id().to_owned(),
                name: if id.is_empty() { "created" } else { "acct-1" }.to_owned(),
                field_values: fields,
                allow_hosts: Vec::new(),
            })
        };

        service
            .upsert_account(request("acct-1", HashMap::new()))
            .await
            .expect("BUG: empty fields preserve the stored value");
        service
            .upsert_account(request(
                "acct-1",
                HashMap::from([("token".to_owned(), "s3cr3t".to_owned())]),
            ))
            .await
            .expect("BUG: identical fields are a no-op");
        service
            .upsert_account(request(
                "",
                HashMap::from([("token".to_owned(), "new".to_owned())]),
            ))
            .await
            .expect("BUG: account creation must succeed");

        compositor.wait_for_credential_push_count(1).await;
        assert_eq!(pushed_widget_ids(&compositor).len(), 1);
        assert!(!compositor.credential_pushes()[0].changed);
    }

    #[tokio::test]
    async fn account_deletion_refreshes_all_and_retries_only_formerly_bound_widgets() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let first = widget_binding(&["acct-1", "acct-1"]);
        let second = widget_binding(&["acct-1"]);
        let other = widget_binding(&["acct-2"]);
        let expected_changes =
            sorted_widget_changes(&[(&first, true), (&second, true), (&other, false)]);
        let scene = scene_of(SceneKind::Combined, vec![first, second, other]);
        let (service, config, store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1", "acct-2"]).await;

        service
            .remove_account(Request::new("acct-1".to_owned()))
            .await
            .expect("BUG: bound account deletion must succeed");

        compositor.wait_for_credential_push_count(3).await;
        assert_eq!(pushed_widget_changes(&compositor), expected_changes);
        let deleted = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        assert!(!store.read().await.accounts().contains_key(&deleted));
        assert!(
            !bindings_by_account(config.read().await.scenes()).contains_key(&deleted),
            "persisted widgets must no longer bind the deleted account"
        );
    }

    #[tokio::test]
    async fn config_save_failure_keeps_account_bindings_and_credentials_unchanged() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let widget = widget_binding(&["acct-1"]);
        let scene = scene_of(SceneKind::Fullscreen, vec![widget]);
        let (service, config, store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;
        replace_file_with_directory(&tmp.path().join("bmc-config.json"));

        let error = service
            .remove_account(Request::new("acct-1".to_owned()))
            .await
            .expect_err("BUG: deleting through an unwritable config must fail");

        let id = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        assert_eq!(error.code(), Code::Internal);
        assert!(
            bindings_by_account(config.read().await.scenes()).contains_key(&id),
            "failed persistence must not commit cloned binding removal"
        );
        assert!(store.read().await.accounts().contains_key(&id));
        compositor.wait_for_credential_push_count(1).await;
        assert_eq!(pushed_widget_ids(&compositor).len(), 1);
        assert!(!compositor.credential_pushes()[0].changed);
    }

    #[tokio::test]
    async fn secret_save_failure_commits_safe_unbound_state_and_clears_former_target() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let affected = widget_binding(&["acct-1"]);
        let unrelated = widget_binding(&["acct-2"]);
        let affected_key = WidgetInstanceKey::new(affected.id.as_uuid());
        let unrelated_key = WidgetInstanceKey::new(unrelated.id.as_uuid());
        let scene = scene_of(
            SceneKind::Combined,
            vec![affected.clone(), unrelated.clone()],
        );
        let (service, config, store, _coordinator, compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1", "acct-2"]).await;
        let unrelated_before = compositor
            .retained_credentials(unrelated_key)
            .expect("BUG: unrelated retained widget must be seeded");
        replace_file_with_directory(&tmp.path().join(crate::secret_store::SECRETS_FILE_NAME));

        let error = service
            .remove_account(Request::new("acct-1".to_owned()))
            .await
            .expect_err("BUG: deleting through an unwritable secret store must fail");

        let id = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        assert_eq!(error.code(), Code::Internal);
        assert!(!bindings_by_account(config.read().await.scenes()).contains_key(&id));
        assert!(store.read().await.accounts().contains_key(&id));
        compositor.wait_for_credential_push_count(2).await;
        assert_eq!(
            pushed_widget_ids(&compositor),
            sorted_widget_ids(&[&affected, &unrelated])
        );
        let (view, secrets) = compositor
            .retained_credentials(affected_key)
            .expect("BUG: affected retained widget must remain registered");
        assert!(view.is_empty());
        assert!(
            secrets.eq(&CredentialSecrets::default()),
            "former target must retain no credential secrets"
        );
        assert!(
            compositor
                .retained_credentials(unrelated_key)
                .as_ref()
                .is_some_and(|retained| retained.eq(&unrelated_before)),
            "unrelated retained credentials must not be refreshed or changed"
        );
    }

    #[tokio::test]
    async fn an_account_pin_is_stored_then_replaced_wholesale_including_by_nothing() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let (service, _config, store, _coordinator, _compositor) =
            seeded_service(&tmp, vec![], &[]).await;

        let upsert = async |id: &str, hosts: &[&str]| {
            service
                .upsert_account(Request::new(web::UpsertAccountRequest {
                    id: id.to_owned(),
                    type_id: credential::BuiltinType::GenericToken.id().to_owned(),
                    name: "Mine".to_owned(),
                    field_values: HashMap::from([("token".to_owned(), "s3cr3t".to_owned())]),
                    allow_hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
                }))
                .await
                .expect("BUG: a clean upsert must succeed")
                .into_inner()
        };
        let stored = async |id: &str| {
            let id = AccountId::from_str(id).expect("BUG: the returned id parses");
            store.read().await.accounts()[&id].allow_hosts.clone()
        };

        let id = upsert("", &["a.example", "b.example"]).await;
        assert_eq!(stored(&id).await, ["a.example", "b.example"]);

        upsert(&id, &["c.example"]).await;
        assert_eq!(
            stored(&id).await,
            ["c.example"],
            "the list is replaced, not merged — a dropped host must really be gone"
        );

        upsert(&id, &[]).await;
        assert!(
            stored(&id).await.is_empty(),
            "an empty list has to clear the pin, unlike field_values which keeps on empty"
        );
    }

    #[tokio::test]
    async fn removing_a_bound_account_unbinds_it_everywhere_then_deletes_it() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let (scene, _) = scene_binding(&["acct-1"]);
        let (service, config_handle, secret_store, _coordinator, _compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;

        service
            .remove_account(Request::new("acct-1".to_owned()))
            .await
            .expect("BUG: a bound account must now cascade rather than refuse");

        assert!(
            bindings_by_account(config_handle.read().await.scenes()).is_empty(),
            "a surviving binding would point at an account that is gone"
        );
        assert!(secret_store.read().await.accounts().is_empty());
    }

    #[tokio::test]
    async fn removing_an_account_that_is_not_there_leaves_the_bindings_alone() {
        use web::account_management_service_server::AccountManagementService as _;

        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let (scene, _) = scene_binding(&["acct-1"]);
        let (service, config_handle, _store, _coordinator, _compositor) =
            seeded_service(&tmp, vec![scene], &["acct-1"]).await;

        let err = service
            .remove_account(Request::new("acct-gone".to_owned()))
            .await
            .expect_err("BUG: an unknown account must not report success");

        assert_eq!(err.code(), tonic::Code::NotFound);
        assert!(
            !bindings_by_account(config_handle.read().await.scenes()).is_empty(),
            "a delete that found nothing must not cascade over the accounts still bound"
        );
    }

    #[test]
    fn unbinding_clears_every_widget_that_named_the_account() {
        let (first, _) = scene_binding(&["acct-1"]);
        let (second, _) = scene_binding(&["acct-1"]);
        let mut scenes = scenes_of(vec![first, second]);
        let id = AccountId::from_str("acct-1").expect("BUG: non-empty id");

        assert!(unbind_account(&mut scenes, &id));
        assert!(
            !bindings_by_account(&scenes).contains_key(&id),
            "a cascade that leaves one binding behind would dangle it"
        );
    }

    #[test]
    fn unbinding_reaches_every_widget_of_one_scene() {
        // The cascade walks scenes and then the widgets within each.
        // Every other case puts one widget in a scene, so only a combined scene
        // exercises the inner walk.
        let (scene, _) = combined_scene(&[&["acct-1"], &["acct-1"], &["acct-2"]]);
        let mut scenes = scenes_of(vec![scene]);
        let doomed = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        let kept = AccountId::from_str("acct-2").expect("BUG: non-empty id");

        assert!(unbind_account(&mut scenes, &doomed));

        let remaining = bindings_by_account(&scenes);
        assert!(
            !remaining.contains_key(&doomed),
            "stopping at the first widget of a scene would leave the second bound"
        );
        assert!(remaining.contains_key(&kept));
    }

    #[test]
    fn unbinding_leaves_other_accounts_alone() {
        let (scene, _) = scene_binding(&["acct-1", "acct-2"]);
        let mut scenes = scenes_of(vec![scene]);
        let doomed = AccountId::from_str("acct-1").expect("BUG: non-empty id");
        let kept = AccountId::from_str("acct-2").expect("BUG: non-empty id");

        assert!(unbind_account(&mut scenes, &doomed));
        assert!(bindings_by_account(&scenes).contains_key(&kept));
    }

    #[test]
    fn unbinding_an_unbound_account_rewrites_nothing() {
        let (scene, _) = scene_binding(&["acct-1"]);
        let mut scenes = scenes_of(vec![scene]);
        let absent = AccountId::from_str("acct-9").expect("BUG: non-empty id");

        assert!(
            !unbind_account(&mut scenes, &absent),
            "reporting a change would save a config nothing touched"
        );
    }

    /// Scenes of 0..4 widgets, each binding 0..3 slots from a 3-account pool: small
    /// enough for a readable shrunk counterexample, wide enough to reach the
    /// multi-widget and shared-account shapes.
    fn arb_scenes() -> impl Strategy<Value = IndexMap<SceneId, Scene>> {
        let bindings = prop::collection::vec(0_usize..3, 0..3);
        let widget = bindings.prop_map(|accounts| {
            let accounts: Vec<String> = accounts.iter().map(|a| format!("acct-{a}")).collect();
            widget_binding(&accounts.iter().map(String::as_str).collect::<Vec<_>>())
        });
        let scene = prop::collection::vec(widget, 0..4)
            .prop_map(|widgets| scene_of(SceneKind::Combined, widgets));

        prop::collection::vec(scene, 0..3).prop_map(|scenes| {
            scenes
                .into_iter()
                .map(|scene| (scene.id, scene))
                .collect::<IndexMap<_, _>>()
        })
    }

    proptest! {
        /// Every account is reported against exactly the widgets that name it, and no
        /// widget is listed twice for one account, whatever the scene shape.
        #[test]
        fn reported_widgets_match_the_bindings_that_name_them(scenes in arb_scenes()) {
            let bound = bindings_by_account(&scenes);

            for (account, widgets) in &bound {
                let mut unique = widgets.clone();
                unique.sort();
                unique.dedup();
                prop_assert_eq!(&unique.len(), &widgets.len(), "widget listed twice");

                for widget_id in widgets {
                    let names_it = scenes.values().any(|scene| {
                        scene.widgets.values().any(|w| {
                            &w.id.to_string() == widget_id
                                && w.credential_bindings.values().any(|a| a == account)
                        })
                    });
                    prop_assert!(names_it, "reported a widget that does not bind the account");
                }
            }

            // …and nothing that does bind an account is left out.
            for scene in scenes.values() {
                for widget in scene.widgets.values() {
                    for account in widget.credential_bindings.values() {
                        let reported = bound.get(account).is_some_and(|widgets| {
                            widgets.contains(&widget.id.to_string())
                        });
                        prop_assert!(reported, "a bound widget went unreported");
                    }
                }
            }
        }
    }

    #[test]
    fn a_rejected_token_is_the_operators_to_fix() {
        let status = upstream_check_status(&CredentialCheckError::Invalid);

        assert_eq!(status.code(), Code::InvalidArgument);
        assert_eq!(status.message(), GrpcError::BadRequest.to_string());
    }

    #[test]
    fn an_unreachable_pool_does_not_claim_the_token_is_invalid() {
        for err in [
            CredentialCheckError::ServiceNotAvailable,
            CredentialCheckError::Unexpected,
            CredentialCheckError::ClientInitFailed,
        ] {
            let status = upstream_check_status(&err);

            assert_eq!(status.code(), Code::Unavailable, "for {err:?}");
            assert_eq!(
                status.message(),
                GrpcError::CredentialUnverified.to_string(),
                "for {err:?}"
            );
        }
    }

    #[test]
    fn every_upstream_outcome_still_marks_the_token_field() {
        for err in [
            CredentialCheckError::Invalid,
            CredentialCheckError::ServiceNotAvailable,
            CredentialCheckError::Unexpected,
            CredentialCheckError::ClientInitFailed,
        ] {
            let fields = violation_fields(&upstream_check_status(&err));

            assert_eq!(fields, [r#"field_values["token"]"#], "for {err:?}");
        }
    }
}

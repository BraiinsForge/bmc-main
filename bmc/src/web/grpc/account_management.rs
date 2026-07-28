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

use crate::config::ConfigHandle;
use crate::credential;
use crate::data::{Account, AccountId};
use crate::scene::{Scene, SceneId};
use crate::secret_store::SecretStoreHandle;
use crate::web::grpc::GrpcError;
use crate::web::grpc::shared::{FieldViolations, ParseOutput, unchecked_field_violations_status};

const POOL_API_URL: &str = "https://api.braiins.com/pool/v2";
const POOL_API_ENDPOINT: &str = "/user/hashrate/current";
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

        // Lock order forces the scan ahead of the existence check.
        // Both facts are gathered first,
        // and only the reporting order is chosen.
        let bound = bindings_by_account(self.config_handle.read().await.scenes());
        let bound_count = bound.get(&id).map_or(0, Vec::len);
        let exists = self.secret_store.read().await.accounts().contains_key(&id);
        match decide_remove_account(exists, bound_count) {
            RemoveAccountOutcome::NotFound => return Err(Status::not_found("Account not found")),
            RemoveAccountOutcome::Bound(count) => {
                let noun = if count == 1 { "widget" } else { "widgets" };
                return Err(Status::failed_precondition(format!(
                    "account is bound to {count} {noun}; unbind it there first"
                )));
            }
            RemoveAccountOutcome::Remove => {}
        }

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
    if type_id != "braiins-pool" {
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

/// What `RemoveAccount` does once both preconditions are known.
#[derive(Debug, PartialEq, Eq)]
enum RemoveAccountOutcome {
    NotFound,
    /// Refused; the count is widgets, not slots.
    Bound(usize),
    Remove,
}

/// A missing account outranks a stale binding,
/// because `secrets.json` is hand-editable:
/// a binding can name an account that is gone,
/// and "unbind it first" would then send the operator
/// chasing something that no longer exists.
fn decide_remove_account(exists: bool, bound_count: usize) -> RemoveAccountOutcome {
    if !exists {
        RemoveAccountOutcome::NotFound
    } else if bound_count > 0 {
        RemoveAccountOutcome::Bound(bound_count)
    } else {
        RemoveAccountOutcome::Remove
    }
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

    use proptest::prelude::*;
    use tonic_types::FieldViolation;

    use super::*;
    use crate::scene::{SceneKind, Widget, WidgetPlacement, WidgetPosition};

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

    /// A one-widget scene whose slots bind the given accounts, keyed `slot0`, `slot1`, …
    /// A widget whose slots, keyed `slot0`, `slot1`, …, bind the given accounts.
    fn widget_binding(accounts: &[&str]) -> Widget {
        let mut widget = Widget::new(
            uuid::Uuid::nil(),
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

    #[test]
    fn an_existing_unbound_account_is_removed() {
        assert_eq!(decide_remove_account(true, 0), RemoveAccountOutcome::Remove);
    }

    #[test]
    fn an_account_that_was_never_there_is_not_found() {
        assert_eq!(
            decide_remove_account(false, 0),
            RemoveAccountOutcome::NotFound
        );
    }

    #[test]
    fn a_bound_account_is_refused_with_its_widget_count() {
        assert_eq!(
            decide_remove_account(true, 2),
            RemoveAccountOutcome::Bound(2)
        );
    }

    #[test]
    fn a_stale_binding_does_not_mask_a_missing_account() {
        assert_eq!(
            decide_remove_account(false, 1),
            RemoveAccountOutcome::NotFound,
            "a binding left over from a hand-edited store must not read as still bound"
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

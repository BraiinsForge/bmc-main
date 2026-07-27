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

//! v1 → current schema upgrade.
//!
//! v1 stored a closed `{ type: "braiins_pool", authentication: { api_key } }` account
//! inside the config; the current schema stores typed credential instances
//! `{ type_id, field_values }` in the secret store beside it (see [`crate::secret_store`]).
//! The upgrade extracts and reshapes the accounts, then re-parses the rest as current.
//!
//! [`reshape_legacy_account`] is the single account transform. The v0 → current path reuses it via
//! [`reshape_and_collect_accounts`]: v0 carries its accounts as raw JSON of the same pre-typed
//! shape, so both hops share one mapping.

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde_json::{Value, json};
use tracing::warn;

use crate::config::{CONFIG_VERSION, Config};
use crate::data::{Account, AccountId};

/// Upgrade a v1 config document to the current schema.
/// Only the accounts change; the rest is already current-shaped and re-parses directly.
pub(super) fn upgrade(mut document: Value) -> Result<(Config, IndexMap<AccountId, Account>)> {
    let accounts = extract_accounts(&mut document);
    document["version"] = json!(CONFIG_VERSION);
    let config = serde_json::from_value(document)
        .context("v1 config body failed to parse as current after account extraction")?;
    Ok((config, accounts))
}

/// Take the document's `accounts` array out — the current config carries none.
fn extract_accounts(document: &mut Value) -> IndexMap<AccountId, Account> {
    let Some(Value::Array(entries)) = document
        .as_object_mut()
        .and_then(|object| object.remove("accounts"))
    else {
        return IndexMap::new();
    };
    reshape_and_collect_accounts(entries)
}

/// Reshape raw pre-typed account values — v0 and v1 carry the same shape — into the current
/// account map, dropping and logging any entry that doesn't match.
pub(super) fn reshape_and_collect_accounts(accounts: Vec<Value>) -> IndexMap<AccountId, Account> {
    let mut out = IndexMap::with_capacity(accounts.len());
    for (index, raw) in accounts.into_iter().enumerate() {
        let Some(reshaped) = reshape_or_warn(index, &raw) else {
            continue;
        };
        match serde_json::from_value::<Account>(reshaped) {
            Ok(account) => {
                out.insert(account.id.clone(), account);
            }
            Err(err) => {
                warn!(index, error = %err, "legacy account dropped: reshaped value failed to parse");
            }
        }
    }
    out
}

fn reshape_or_warn(index: usize, raw: &Value) -> Option<Value> {
    let reshaped = reshape_legacy_account(raw);
    if reshaped.is_none() {
        warn!(
            index,
            "legacy account dropped: not a recognized braiins_pool account"
        );
    }
    reshaped
}

/// Map a pre-typed-credential (v1) account object to the current typed-credential shape. The v1
/// schema only ever produced `braiins_pool` + `api_key`, mapped to the `braiins-pool` credential
/// type's `token` field. Returns `None` for any other shape.
fn reshape_legacy_account(raw: &Value) -> Option<Value> {
    let object = raw.as_object()?;
    if object.get("type")?.as_str()? != "braiins_pool" {
        return None;
    }
    let id = object.get("id")?.clone();
    let name = object.get("name")?.clone();
    let created_at = object.get("created_at")?.clone();
    let token = object.get("authentication")?.get("api_key")?.as_str()?;
    Some(json!({
        "id": id,
        "type_id": "braiins-pool",
        "name": name,
        "field_values": { "token": token },
        "created_at": created_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reshapes_braiins_pool_account() {
        let v1 = json!({
            "id": "abc",
            "type": "braiins_pool",
            "name": "primary",
            "authentication": { "api_key": "sk-1" },
            "created_at": "2025-01-01T00:00:00Z"
        });
        let out = reshape_legacy_account(&v1).expect("BUG: braiins_pool account should reshape");
        assert_eq!(out["id"], "abc");
        assert_eq!(out["type_id"], "braiins-pool");
        assert_eq!(out["name"], "primary");
        assert_eq!(out["field_values"]["token"], "sk-1");
        assert!(
            out.get("authentication").is_none(),
            "secret nesting is dropped"
        );
    }

    #[test]
    fn drops_unknown_account_type() {
        let v1 = json!({
            "id": "x", "type": "something_else", "name": "n",
            "authentication": { "api_key": "k" }, "created_at": "2025-01-01T00:00:00Z"
        });
        assert!(reshape_legacy_account(&v1).is_none());
    }

    #[test]
    fn drops_account_without_api_key() {
        let v1 = json!({
            "id": "x", "type": "braiins_pool", "name": "n",
            "authentication": {}, "created_at": "2025-01-01T00:00:00Z"
        });
        assert!(reshape_legacy_account(&v1).is_none());
    }

    #[test]
    fn upgrade_extracts_accounts_and_pins_version() {
        let document = json!({
            "version": 1,
            "scenes": [],
            "accounts": [{
                "id": "a", "type": "braiins_pool", "name": "p",
                "authentication": { "api_key": "tok" }, "created_at": "2025-01-01T00:00:00Z"
            }]
        });
        let (config, accounts) = upgrade(document).expect("BUG: v1 document should upgrade");
        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(accounts.len(), 1);
        let account = accounts.values().next().expect("BUG: one account");
        assert_eq!(account.type_id, "braiins-pool");
        assert_eq!(account.field_values["token"], "tok");

        let serialized =
            serde_json::to_value(&config).expect("BUG: upgraded config must serialize");
        assert!(
            serialized.get("accounts").is_none(),
            "the config must no longer carry accounts"
        );
    }
}

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

use bmc_field_schema::ParamKey;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::time::Duration;
use tracing::warn;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneCycling {
    pub automatic_cycling_enabled: bool,
    #[serde(with = "humantime_serde")]
    pub automatic_cycling_default_duration: Duration,
    pub transition: SceneCyclingTransition,
}

impl Default for SceneCycling {
    fn default() -> Self {
        Self {
            automatic_cycling_enabled: true,
            automatic_cycling_default_duration: Duration::from_secs(30),
            transition: SceneCyclingTransition::Slide,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SceneCyclingTransition {
    Slide,
    Fade,
    /// Instant scene switch with no animation.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct AccountId(String);

impl AccountId {
    fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Display for AccountId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ParseAccountIdError;

impl Display for ParseAccountIdError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("account id cannot be empty")
    }
}

impl Error for ParseAccountIdError {}

impl FromStr for AccountId {
    type Err = ParseAccountIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Err(ParseAccountIdError)
        } else {
            Ok(Self(value.to_owned()))
        }
    }
}

/// A saved account — a typed instance of a credential type (see [`crate::credential`]).
/// `type_id` names the credential type; `field_values` are the secret values, keyed by field key.
#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub type_id: String,
    pub name: String,
    pub field_values: IndexMap<ParamKey, String>,
    /// Hosts this account's secret may be sent to, in the egress-pin grammar.
    /// Non-empty, it is the authoritative pin for the account — deliberately,
    /// even if the type carries one: whoever writes this store owns the device.
    /// Empty defers to the type's pin, or to none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_hosts: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Hand-written so a debug-logged account can never
/// leak its secrets; logs end up in support archives.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("type_id", &self.type_id)
            .field("name", &self.name)
            .field("field_values", &RedactedKeys(&self.field_values))
            .field("allow_hosts", &self.allow_hosts)
            .field("created_at", &self.created_at)
            .finish()
    }
}

struct RedactedKeys<'a>(&'a IndexMap<ParamKey, String>);

impl std::fmt::Debug for RedactedKeys<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|key| (key.as_str(), "<redacted>")))
            .finish()
    }
}

impl Account {
    #[must_use]
    pub fn new(type_id: String, name: String, field_values: IndexMap<ParamKey, String>) -> Self {
        Self {
            id: AccountId::generate(),
            type_id,
            name,
            field_values,
            allow_hosts: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

#[inline]
pub fn serialize_accounts<S: Serializer>(
    map: &IndexMap<AccountId, Account>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(map.values())
}

/// Deserialize the on-disk account array, keyed by id. Entries that don't match the current
/// schema (e.g. a pre-typed-credential account) are dropped with a warning rather than failing
/// the whole config load.
pub fn deserialize_accounts<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<IndexMap<AccountId, Account>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    let mut map = IndexMap::with_capacity(raw.len());
    for (index, value) in raw.into_iter().enumerate() {
        match serde_json::from_value::<Account>(value) {
            Ok(account) => {
                map.insert(account.id.clone(), account);
            }
            Err(err) => {
                warn!(index, error = %err, "dropping account that does not match the current schema");
            }
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scene_cycling_default_matches_existing_config_defaults() {
        let config = SceneCycling::default();

        assert!(config.automatic_cycling_enabled);
        assert_eq!(
            config.automatic_cycling_default_duration,
            Duration::from_secs(30)
        );
        assert_eq!(config.transition, SceneCyclingTransition::Slide);
    }

    #[test]
    fn scene_cycling_transition_serializes_snake_case() {
        for (transition, expected) in [
            (SceneCyclingTransition::Slide, json!("slide")),
            (SceneCyclingTransition::Fade, json!("fade")),
            (SceneCyclingTransition::None, json!("none")),
        ] {
            let serialized = serde_json::to_value(transition)
                .expect("BUG: transition serialization must succeed");
            assert_eq!(serialized, expected);
            let roundtrip: SceneCyclingTransition = serde_json::from_value(serialized)
                .expect("BUG: transition deserialization must succeed");
            assert_eq!(roundtrip, transition);
        }
    }

    #[test]
    fn account_id_rejects_empty_string() {
        let result = AccountId::from_str("");

        assert_eq!(result.err(), Some(ParseAccountIdError));
    }

    #[test]
    fn deserialize_accounts_indexes_by_id_and_drops_incompatible() {
        let valid_id = Uuid::new_v4().to_string();
        let json = json!([
            {
                "id": valid_id,
                "type_id": "braiins-pool",
                "name": "primary",
                "field_values": { "token": "a-token" },
                "created_at": "2025-01-01T00:00:00Z"
            },
            {
                // legacy pre-typed-credential shape — dropped, not fatal
                "id": Uuid::new_v4().to_string(),
                "type": "braiins_pool",
                "name": "legacy",
                "authentication": { "api_key": "old-key" },
                "created_at": "2025-01-02T00:00:00Z"
            }
        ]);

        let accounts = deserialize_accounts(json).expect("BUG: valid array should deserialize");

        assert_eq!(accounts.len(), 1);
        let id = AccountId::from_str(&valid_id).expect("BUG: id is valid");
        assert_eq!(accounts[&id].type_id, "braiins-pool");
        assert_eq!(accounts[&id].field_values["token"], "a-token");
    }
}

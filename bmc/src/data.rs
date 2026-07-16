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

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::hash::Hash;
use std::str::FromStr;
use std::time::Duration;
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

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SceneCyclingTransition {
    Slide,
    Fade,
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

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    BraiinsPool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationType {
    ApiKey(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub r#type: AccountType,
    pub name: String,
    pub authentication: AuthenticationType,
    pub created_at: DateTime<Utc>,
}

impl Account {
    #[must_use]
    pub fn new(account_type: AccountType, name: &str, authentication: AuthenticationType) -> Self {
        Self {
            id: AccountId::generate(),
            r#type: account_type,
            name: name.to_owned(),
            authentication,
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

#[inline]
pub fn deserialize_accounts<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<IndexMap<AccountId, Account>, D::Error> {
    de_indexmap(deserializer, |account: &Account| account.id.clone())
}

fn de_indexmap<'de, D: Deserializer<'de>, K: Hash + Eq, V: Deserialize<'de>>(
    deserializer: D,
    key_selector: impl Fn(&V) -> K,
) -> Result<IndexMap<K, V>, D::Error> {
    let vec = Vec::<V>::deserialize(deserializer)?;
    let map = vec
        .into_iter()
        .map(|value| (key_selector(&value), value))
        .collect::<IndexMap<_, _>>();

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
    fn account_id_rejects_empty_string() {
        let result = AccountId::from_str("");

        assert_eq!(result.err(), Some(ParseAccountIdError));
    }

    #[test]
    fn deserialize_accounts_indexes_by_account_id() {
        let first_id = Uuid::new_v4().to_string();
        let second_id = Uuid::new_v4().to_string();
        let json = json!([
            {
                "id": first_id,
                "type": "braiins_pool",
                "name": "primary",
                "authentication": { "api_key": "first-key" },
                "created_at": "2025-01-01T00:00:00Z"
            },
            {
                "id": second_id,
                "type": "braiins_pool",
                "name": "backup",
                "authentication": { "api_key": "second-key" },
                "created_at": "2025-01-02T00:00:00Z"
            }
        ]);

        let accounts = deserialize_accounts(json).expect("BUG: test JSON should deserialize");

        assert_eq!(accounts.len(), 2);
        assert!(accounts.contains_key(&AccountId::from_str(&first_id).expect("BUG: id is valid")));
        assert!(accounts.contains_key(&AccountId::from_str(&second_id).expect("BUG: id is valid")));
    }
}

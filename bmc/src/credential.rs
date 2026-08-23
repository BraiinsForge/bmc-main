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

//! Firmware-side view of the credential-type catalog defined
//! in [`bmc_field_schema::credential`], plus the rule
//! for which of a widget's bindings actually count.

use std::collections::BTreeMap;

use bmc_widget_manifest::{CredentialKey, CredentialSlot};
use bmc_widget_protocol::CredentialSecrets;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::data::{Account, AccountId};

pub use bmc_field_schema::credential::*;

/// Both halves of a resolved binding set,
/// produced together so the account lookup runs once.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolution {
    /// Slot → `{"type": …, "account": …}`,
    /// the whole of what the guest may learn.
    pub view: Map<String, Value>,
    pub secrets: CredentialSecrets,
}

/// Why the installed manifest no longer authorises a stored binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unauthorised {
    SlotUndeclared,
    TypeMismatch,
}

impl Unauthorised {
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::SlotUndeclared => "the installed manifest no longer declares this slot",
            Self::TypeMismatch => "the slot is now declared with a different credential type",
        }
    }
}

/// Split stored bindings by whether the installed manifest still authorises them.
///
/// The slots a widget can ever receive are fixed by its manifest,
/// and that has to hold for config written earlier:
/// a package can update under the same uid and drop a slot or redeclare its type,
/// and nothing re-runs the write-path validation over config already on disk.
#[must_use]
pub fn authorised_bindings(
    bindings: &BTreeMap<CredentialKey, AccountId>,
    slots: &IndexMap<CredentialKey, CredentialSlot>,
    accounts: &IndexMap<AccountId, Account>,
) -> (
    BTreeMap<CredentialKey, AccountId>,
    Vec<(CredentialKey, Unauthorised)>,
) {
    let mut authorised = BTreeMap::new();
    let mut rejected = Vec::new();
    for (slot, id) in bindings {
        match (slots.get(slot), accounts.get(id)) {
            (None, _) => rejected.push((slot.clone(), Unauthorised::SlotUndeclared)),
            (Some(declared), Some(account)) if account.type_id != declared.type_id => {
                rejected.push((slot.clone(), Unauthorised::TypeMismatch));
            }
            // A binding naming an account that is gone cannot be type-checked;
            // it already reads as unbound through `effective_bindings`.
            (Some(_), _) => {
                authorised.insert(slot.clone(), id.clone());
            }
        }
    }
    (authorised, rejected)
}

/// Bindings whose account still exists, in slot order.
///
/// A binding naming a missing account is not a binding.
/// `secrets.json` is a plain file an operator can edit,
/// so config outliving the account it names is a reachable
/// state rather than a corruption to repair.
///
/// Every reader agrees on this subset, so such a slot reads
/// as unbound and its widget degrades instead of pointing at nothing.
pub fn effective_bindings<'a>(
    bindings: &'a BTreeMap<CredentialKey, AccountId>,
    accounts: &'a IndexMap<AccountId, Account>,
) -> impl Iterator<Item = (&'a CredentialKey, &'a Account)> {
    bindings
        .iter()
        .filter_map(|(slot, id)| accounts.get(id).map(|account| (slot, account)))
}

/// Slots bound to an account that is gone,
/// for the caller to report with its widget context.
pub fn dangling_bindings<'a>(
    bindings: &'a BTreeMap<CredentialKey, AccountId>,
    accounts: &'a IndexMap<AccountId, Account>,
) -> impl Iterator<Item = (&'a CredentialKey, &'a AccountId)> {
    bindings
        .iter()
        .filter(|(_, id)| !accounts.contains_key(*id))
}

#[must_use]
pub fn resolve(
    bindings: &BTreeMap<CredentialKey, AccountId>,
    accounts: &IndexMap<AccountId, Account>,
) -> Resolution {
    let mut view = Map::new();
    let mut secrets = Map::new();

    for (slot, account) in effective_bindings(bindings, accounts) {
        view.insert(
            slot.as_str().to_owned(),
            serde_json::json!({ "type": account.type_id, "account": account.name }),
        );
        let fields: Map<String, Value> = account
            .field_values
            .iter()
            .map(|(field, value)| (field.as_str().to_owned(), Value::String(value.clone())))
            .collect();
        let mut slot_value = serde_json::json!({ "fields": fields });
        if !account.allow_hosts.is_empty() {
            slot_value["allow_hosts"] = serde_json::json!(account.allow_hosts);
        }
        secrets.insert(slot.as_str().to_owned(), slot_value);
    }

    Resolution {
        view,
        secrets: CredentialSecrets::new(secrets),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn slot(key: &str) -> CredentialKey {
        CredentialKey::try_new(key.to_owned()).expect("BUG: identifier-shaped slot key")
    }

    fn account_id(id: &str) -> AccountId {
        AccountId::from_str(id).expect("BUG: non-empty id")
    }

    /// Slot keys for the widget these tests bind against.
    /// Arbitrary names, shared so a reader sees the same two slots throughout.
    const POOL: &str = "pool";
    const SPARE: &str = "spare";

    fn account(id: &str, type_id: &str, name: &str, field: &str, value: &str) -> Account {
        let mut field_values = IndexMap::new();
        field_values.insert(slot(field), value.to_owned());

        Account {
            id: account_id(id),
            type_id: type_id.to_owned(),
            name: name.to_owned(),
            field_values,
            allow_hosts: Vec::new(),
            created_at: chrono::Utc::now(),
        }
    }

    fn store(accounts: Vec<Account>) -> IndexMap<AccountId, Account> {
        accounts
            .into_iter()
            .map(|account| (account.id.clone(), account))
            .collect()
    }

    fn bound(pairs: &[(&str, &str)]) -> BTreeMap<CredentialKey, AccountId> {
        pairs
            .iter()
            .map(|(s, id)| (slot(s), account_id(id)))
            .collect()
    }

    fn pool_account() -> Account {
        account(
            "a-1",
            BuiltinType::BraiinsPool.id(),
            "My pool",
            "token",
            "s3cr3t",
        )
    }

    fn declares(pairs: &[(&str, &str)]) -> IndexMap<CredentialKey, CredentialSlot> {
        pairs
            .iter()
            .map(|(key, type_id)| {
                (
                    slot(key),
                    CredentialSlot {
                        type_id: (*type_id).to_owned(),
                        label: "Pool account".to_owned(),
                        description: None,
                        required: false,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_slot_the_manifest_still_declares_is_authorised() {
        let (authorised, rejected) = authorised_bindings(
            &bound(&[(POOL, "a-1")]),
            &declares(&[(POOL, BuiltinType::BraiinsPool.id())]),
            &store(vec![pool_account()]),
        );

        assert_eq!(authorised, bound(&[(POOL, "a-1")]));
        assert!(rejected.is_empty());
    }

    #[test]
    fn a_slot_the_updated_manifest_dropped_is_withheld() {
        let (authorised, rejected) = authorised_bindings(
            &bound(&[(POOL, "a-1")]),
            &declares(&[]),
            &store(vec![pool_account()]),
        );

        assert!(
            authorised.is_empty(),
            "a widget must not be handed a secret for a slot it no longer declares"
        );
        assert_eq!(rejected, vec![(slot(POOL), Unauthorised::SlotUndeclared)]);
    }

    #[test]
    fn a_slot_redeclared_with_another_type_is_withheld() {
        let (authorised, rejected) = authorised_bindings(
            &bound(&[(POOL, "a-1")]),
            &declares(&[(POOL, BuiltinType::GenericToken.id())]),
            &store(vec![pool_account()]),
        );

        assert!(
            authorised.is_empty(),
            "a pool token must not satisfy a slot redeclared as a generic one"
        );
        assert_eq!(rejected, vec![(slot(POOL), Unauthorised::TypeMismatch)]);
    }

    /// A missing account cannot be type-checked. Withholding it here would report it
    /// as a manifest problem, when it is the already-handled unbound case.
    #[test]
    fn a_binding_whose_account_vanished_is_left_to_the_unbound_path() {
        let (authorised, rejected) = authorised_bindings(
            &bound(&[(POOL, "a-1")]),
            &declares(&[(POOL, BuiltinType::BraiinsPool.id())]),
            &store(vec![]),
        );

        assert_eq!(authorised, bound(&[(POOL, "a-1")]));
        assert!(rejected.is_empty());
        assert_eq!(effective(&authorised, &store(vec![])).len(), 0);
    }

    fn effective(
        bindings: &BTreeMap<CredentialKey, AccountId>,
        accounts: &IndexMap<AccountId, Account>,
    ) -> Vec<(String, String)> {
        effective_bindings(bindings, accounts)
            .map(|(slot, account)| (slot.as_str().to_owned(), account.name.clone()))
            .collect()
    }

    #[test]
    fn a_bound_slot_yields_its_account() {
        let bindings = bound(&[(POOL, "a-1")]);

        assert_eq!(
            effective(&bindings, &store(vec![pool_account()])),
            vec![(POOL.to_owned(), "My pool".to_owned())]
        );
    }

    #[test]
    fn an_unbound_widget_yields_nothing() {
        assert!(effective(&BTreeMap::new(), &store(vec![pool_account()])).is_empty());
    }

    #[test]
    fn a_binding_naming_a_missing_account_counts_as_unbound() {
        assert!(
            effective(&bound(&[(POOL, "gone")]), &store(vec![])).is_empty(),
            "a hand-edited store must leave the slot unbound, not half-resolved"
        );
    }

    #[test]
    fn a_missing_account_does_not_hide_the_slots_around_it() {
        let bindings = bound(&[(POOL, "a-1"), (SPARE, "gone")]);

        assert_eq!(
            effective(&bindings, &store(vec![pool_account()])),
            vec![(POOL.to_owned(), "My pool".to_owned())]
        );
    }

    #[test]
    fn one_account_on_two_slots_yields_both() {
        let bindings = bound(&[(POOL, "a-1"), (SPARE, "a-1")]);
        let yielded = effective(&bindings, &store(vec![pool_account()]));

        assert_eq!(
            yielded.iter().map(|(slot, _)| slot).collect::<Vec<_>>(),
            vec![POOL, SPARE]
        );
    }

    #[test]
    fn resolve_carries_the_accounts_own_pin_and_omits_an_empty_one() {
        let mut pinned = pool_account();
        pinned.allow_hosts = vec!["api.example.com".to_owned()];
        let plain = account("a-2", BuiltinType::GenericToken.id(), "T", "token", "x");
        let bindings = bound(&[(POOL, "a-1"), (SPARE, "a-2")]);

        let resolution = resolve(&bindings, &store(vec![pinned, plain]));

        assert_eq!(
            resolution.secrets.allow_hosts(POOL),
            vec!["api.example.com"]
        );
        let wire: serde_json::Value = serde_json::from_str(&resolution.secrets.to_json_string())
            .expect("BUG: the secrets payload must be valid JSON");
        assert!(
            wire[SPARE].get("allow_hosts").is_none(),
            "an account without a pin must not grow an empty list on the wire"
        );
        assert_eq!(
            resolution.secrets.field(POOL, "token"),
            Some("s3cr3t"),
            "field access must reach through the nested shape"
        );
        assert!(
            !serde_json::Value::Object(resolution.view.clone())
                .to_string()
                .contains("allow_hosts"),
            "the guest-visible view must not carry the pin"
        );
    }
}

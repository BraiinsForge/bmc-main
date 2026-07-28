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

use bmc_widget_manifest::CredentialKey;
use indexmap::IndexMap;

use crate::data::{Account, AccountId};

pub use bmc_field_schema::credential::*;

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

    fn account(id: &str, type_id: &str, name: &str, field: &str, value: &str) -> Account {
        let mut field_values = IndexMap::new();
        field_values.insert(slot(field), value.to_owned());

        Account {
            id: account_id(id),
            type_id: type_id.to_owned(),
            name: name.to_owned(),
            field_values,
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
        account("a-1", "braiins-pool", "My pool", "token", "s3cr3t")
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
        let bindings = bound(&[("pool", "a-1")]);

        assert_eq!(
            effective(&bindings, &store(vec![pool_account()])),
            vec![("pool".to_owned(), "My pool".to_owned())]
        );
    }

    #[test]
    fn an_unbound_widget_yields_nothing() {
        assert!(effective(&BTreeMap::new(), &store(vec![pool_account()])).is_empty());
    }

    #[test]
    fn a_binding_naming_a_missing_account_counts_as_unbound() {
        assert!(
            effective(&bound(&[("pool", "gone")]), &store(vec![])).is_empty(),
            "a hand-edited store must leave the slot unbound, not half-resolved"
        );
    }

    #[test]
    fn a_missing_account_does_not_hide_the_slots_around_it() {
        let bindings = bound(&[("pool", "a-1"), ("spare", "gone")]);

        assert_eq!(
            effective(&bindings, &store(vec![pool_account()])),
            vec![("pool".to_owned(), "My pool".to_owned())]
        );
    }

    #[test]
    fn one_account_on_two_slots_yields_both() {
        let bindings = bound(&[("pool", "a-1"), ("spare", "a-1")]);
        let yielded = effective(&bindings, &store(vec![pool_account()]));

        assert_eq!(
            yielded.iter().map(|(slot, _)| slot).collect::<Vec<_>>(),
            vec!["pool", "spare"]
        );
    }
}

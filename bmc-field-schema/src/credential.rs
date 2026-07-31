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

//! Credential-type schema — a named set of secret fields, built on this crate's [`ParamDefinition`]
//! so one form renderer drives both params and credential fields.
//!
//! Here rather than in the firmware crate so `bmc-widget-codegen` can read each type's fields.

use std::net::{IpAddr, Ipv6Addr};
use std::sync::LazyLock;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{ParamDefinition, ParamKey, ParamKind, StringFormat};

/// A kind of account a widget can bind, e.g. a Braiins Pool API token.
/// Each field key is the interpolation variable a widget embeds
/// as `{{ credential.<slot>.<field_key> }}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialType {
    /// Stable id, referenced by widget manifests and accounts.
    pub id: String,
    pub name: String,
    pub description: String,
    /// Keyed by interpolation variable; secret fields carry [`StringFormat::Password`].
    pub fields: IndexMap<ParamKey, ParamDefinition>,
    /// Absent means the secret may be sent anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<EgressPolicy>,
    /// Absent means the frontend renders its own generic glyph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
}

/// Artwork carried as bytes rather than as a path or URL.
///
/// A built-in has no directory to be served from the way a widget package does,
/// and a type admitted from elsewhere would have a different one again.
/// Carrying the bytes makes every source look the same to a reader,
/// whether they were baked into the binary, encoded at build time or read from disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Icon {
    /// IANA media type of `data`, e.g. `image/svg+xml`.
    pub mime_type: String,
    /// Base64 of the icon bytes, so any renderable format travels unchanged.
    pub data: String,
}

/// Hosts a credential type's secrets may be sent to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EgressPolicy {
    /// Where this type's secret may go.
    /// Each entry is one of:
    ///
    /// - an exact host, optionally with a port
    ///     - `api.example.com`
    ///     - `api.example.com:8443`.
    ///     - Omitting the port allows any port
    /// - a subdomain wildcard `*.example.com`, matching exactly one label
    ///   and never the apex, as TLS and cookies do;
    /// - a CIDR range `10.0.0.0/8`, `fd00::/8`;
    /// - an IP address, in any equivalent spelling; IPv6 is written bare
    ///   (`fd00::1`) or bracketed, and brackets are how a port attaches:
    ///   `[fd00::1]:8443`.
    ///
    /// An empty list allows everything, exactly as omitting the policy does.
    pub allow_hosts: Vec<String>,
}

impl EgressPolicy {
    /// Whether this policy permits sending the secret to `host`.
    ///
    /// `host` is the request authority with any IPv6 brackets already removed,
    /// and `port` its explicit port if the URL carried one.
    /// The caller does that split because it holds a URL parser;
    /// this crate deliberately has no URL dependency.
    ///
    /// Comparison is ASCII-case-insensitive.
    /// A non-ASCII host is compared as-is,
    /// so an internationalised domain must be listed in punycode.
    #[must_use]
    pub fn allows(&self, host: &str, port: Option<u16>) -> bool {
        self.allow_hosts.is_empty()
            || self
                .allow_hosts
                .iter()
                .any(|entry| entry_allows(entry, host, port))
    }
}

fn entry_allows(entry: &str, host: &str, port: Option<u16>) -> bool {
    if entry.contains('/') {
        // A range can only speak about literal addresses.
        // Resolving a name here would approve one address
        // and let the fetch dial another.
        return match (entry.parse::<ipnet::IpNet>(), host.parse::<IpAddr>()) {
            (Ok(network), Ok(addr)) => network.contains(&addr),
            _ => false,
        };
    }

    let (entry_host, entry_port) = split_port(entry);
    if entry_port.is_some() && entry_port != port {
        return false;
    }
    match entry_host.strip_prefix("*.") {
        // Lowercased on both sides: `strip_suffix` is case-sensitive,
        // entries are operator-written, and the exact arm below already
        // ignores case through `eq_ignore_ascii_case`.
        Some(suffix) => host
            .to_ascii_lowercase()
            .strip_suffix(&suffix.to_ascii_lowercase())
            .and_then(|label| label.strip_suffix('.'))
            // One label, and never the apex: an empty prefix
            // would mean the bare domain,
            // a dotted one would reach deeper than the entry declared.
            .is_some_and(|label| !label.is_empty() && !label.contains('.')),
        // Numerically as well as textually: the request host arrives canonicalised,
        // and an operator's equivalent IPv6 spelling must not die on the difference.
        None => {
            entry_host.eq_ignore_ascii_case(host)
                || matches!(
                    (entry_host.parse::<IpAddr>(), host.parse::<IpAddr>()),
                    (Ok(entry_addr), Ok(host_addr)) if entry_addr == host_addr
                )
        }
    }
}

/// Split a trailing `:port`, ignoring the colons inside an IPv6 literal.
/// A bracketed literal comes back unbracketed, matching the bare form
/// the caller extracts from the request URL.
/// A malformed bracket spelling comes back whole, and so matches nothing.
fn split_port(entry: &str) -> (&str, Option<u16>) {
    if let Some(inner) = entry.strip_prefix('[') {
        return match inner.split_once(']') {
            Some((host, "")) => (host, None),
            Some((host, tail)) => match tail.strip_prefix(':') {
                Some(port) => (host, port.parse().ok()),
                None => (entry, None),
            },
            None => (entry, None),
        };
    }
    match entry.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, port.parse().ok()),
        _ => (entry, None),
    }
}

/// Why an operator-written egress entry is unusable, in words the form can echo.
///
/// [`entry_allows`] never fails — an entry that matches nothing
/// simply allows nothing — so saving runs this instead,
/// rejecting a line that would sit in the list silently dead.
///
/// # Errors
///
/// A static English sentence naming what is wrong with the entry.
pub fn check_entry(entry: &str) -> Result<(), &'static str> {
    if entry.is_empty() {
        return Err("an entry cannot be empty");
    }
    if entry.chars().any(char::is_whitespace) {
        return Err("an entry cannot contain spaces");
    }
    if entry.contains('/') {
        return match entry.parse::<ipnet::IpNet>() {
            Ok(_) => Ok(()),
            Err(_) => Err("\"/\" makes this a CIDR range, and it does not parse as one"),
        };
    }
    if entry.contains('[') || entry.contains(']') {
        return check_bracketed_entry(entry);
    }

    let (host, port) = split_port(entry);
    // The matcher drops a `:suffix` that is not a port, then matches
    // the bare host on any port; a save must not inherit that lenience.
    if port.is_none() && host != entry {
        return Err("the part after \":\" is not a port number");
    }
    if host.is_empty() {
        return Err("there is no host before the \":\"");
    }
    match host.strip_prefix("*.") {
        Some(suffix) if suffix.is_empty() || suffix.starts_with('.') => {
            Err("a wildcard needs a domain right after \"*.\"")
        }
        Some(suffix) if suffix.contains('*') => {
            Err("one leading \"*.\" is the only wildcard there is")
        }
        None if host.contains('*') => Err("\"*\" is only valid as a leading \"*.\" label"),
        Some(_) | None => Ok(()),
    }
}

/// The bracketed spelling is IPv6-only,
/// and the sole way to pin an IPv6 address to a port.
fn check_bracketed_entry(entry: &str) -> Result<(), &'static str> {
    let Some(inner) = entry.strip_prefix('[') else {
        return Err("brackets can only open an IPv6 literal");
    };
    let Some((host, tail)) = inner.split_once(']') else {
        return Err("the \"[\" is never closed");
    };
    if host.parse::<Ipv6Addr>().is_err() {
        return Err("brackets must hold an IPv6 address");
    }
    match tail.strip_prefix(':') {
        None if tail.is_empty() => Ok(()),
        None => Err("only \":port\" may follow the \"]\""),
        Some(port) => match port.parse::<u16>() {
            Ok(_) => Ok(()),
            Err(_) => Err("the part after \":\" is not a port number"),
        },
    }
}

/// Storage and the wire keep a plain string id, because user-defined types are planned
/// and a string is what those will carry; an id matching no variant is simply not a built-in.
///
/// Naming the known ones anyway keeps [`builtins`] derived from [`Self::ALL`],
/// so a new type cannot be left out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinType {
    GenericToken,
    GenericUserpass,
    BraiinsPool,
}

impl BuiltinType {
    /// In catalog order: [`builtins`] preserves it, and the picker follows.
    pub const ALL: [Self; 3] = [Self::GenericToken, Self::GenericUserpass, Self::BraiinsPool];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::GenericToken => "generic-token",
            Self::GenericUserpass => "generic-userpass",
            Self::BraiinsPool => "braiins-pool",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|builtin| builtin.id() == id)
    }

    #[must_use]
    pub fn schema(self) -> CredentialType {
        match self {
            Self::GenericToken => generic_token(),
            Self::GenericUserpass => generic_userpass(),
            Self::BraiinsPool => braiins_pool(),
        }
    }

    #[must_use]
    pub fn egress(self) -> Option<EgressPolicy> {
        self.schema().egress
    }
}

/// The fixed set of firmware-provided credential types.
#[must_use]
pub fn builtins() -> Vec<CredentialType> {
    BuiltinType::ALL
        .into_iter()
        .map(BuiltinType::schema)
        .collect()
}

/// Encode a checked-in SVG for the wire.
///
/// The asset stays a plain file, so a change to it shows up
/// as artwork in review rather than as a wall of base64.
fn svg_icon(source: &str) -> Icon {
    use base64::Engine as _;
    Icon {
        mime_type: "image/svg+xml".to_owned(),
        data: base64::engine::general_purpose::STANDARD.encode(source),
    }
}

/// Encoded once rather than per [`BuiltinType::schema`] call:
/// [`BuiltinType::egress`] builds a whole schema to read one field,
/// and that runs for every outbound request spending a credential.
static BRAIINS_POOL_ICON: LazyLock<Icon> =
    LazyLock::new(|| svg_icon(include_str!("../assets/braiins-pool.svg")));
static GENERIC_TOKEN_ICON: LazyLock<Icon> =
    LazyLock::new(|| svg_icon(include_str!("../assets/generic-token.svg")));
static GENERIC_USERPASS_ICON: LazyLock<Icon> =
    LazyLock::new(|| svg_icon(include_str!("../assets/generic-userpass.svg")));

fn secret_field(name: &str, description: &str) -> ParamDefinition {
    string_field(name, description, Some(StringFormat::Password))
}

fn string_field(name: &str, description: &str, format: Option<StringFormat>) -> ParamDefinition {
    ParamDefinition {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        is_optional: false,
        kind: ParamKind::String {
            format,
            enum_values: Vec::new(),
            default_value: None,
        },
    }
}

fn field_map<const N: usize>(
    entries: [(&str, ParamDefinition); N],
) -> IndexMap<ParamKey, ParamDefinition> {
    entries
        .into_iter()
        .map(|(key, def)| {
            let key = ParamKey::try_new(key.to_owned())
                .expect("BUG: builtin credential field key must be a valid ParamKey");
            (key, def)
        })
        .collect()
}

fn generic_token() -> CredentialType {
    CredentialType {
        id: BuiltinType::GenericToken.id().to_owned(),
        name: "Token".to_owned(),
        description: "A single API token or bearer secret.".to_owned(),
        fields: field_map([(
            "token",
            secret_field("Token", "The API token or bearer secret."),
        )]),
        egress: None,
        icon: Some(GENERIC_TOKEN_ICON.clone()),
    }
}

fn generic_userpass() -> CredentialType {
    CredentialType {
        id: BuiltinType::GenericUserpass.id().to_owned(),
        name: "Username & password".to_owned(),
        description: "A username and password pair.".to_owned(),
        fields: field_map([
            (
                "username",
                string_field("Username", "The account username.", None),
            ),
            (
                "password",
                secret_field("Password", "The account password."),
            ),
        ]),
        egress: None,
        icon: Some(GENERIC_USERPASS_ICON.clone()),
    }
}

/// The one host a Braiins Pool token may reach.
///
/// Shared so the egress pin and the firmware's own token check cannot drift:
/// moving one alone leaves the pin refusing the very API it exists to permit.
pub const BRAIINS_POOL_HOST: &str = "api.braiins.com";

fn braiins_pool() -> CredentialType {
    CredentialType {
        id: BuiltinType::BraiinsPool.id().to_owned(),
        name: "Braiins Pool".to_owned(),
        description: "A Braiins Pool API token used to fetch your worker stats.".to_owned(),
        fields: field_map([(
            "token",
            secret_field("API token", "Your Braiins Pool API token."),
        )]),
        egress: Some(EgressPolicy {
            allow_hosts: vec![BRAIINS_POOL_HOST.to_owned()],
        }),
        icon: Some(BRAIINS_POOL_ICON.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(entries: &[&str]) -> EgressPolicy {
        EgressPolicy {
            allow_hosts: entries.iter().map(|e| (*e).to_owned()).collect(),
        }
    }

    #[test]
    fn an_empty_list_allows_everything() {
        assert!(policy(&[]).allows("anywhere.example.com", None));
    }

    #[test]
    fn an_exact_host_matches_regardless_of_case_or_port() {
        let pinned = policy(&["api.braiins.com"]);

        assert!(pinned.allows("api.braiins.com", None));
        assert!(pinned.allows("API.Braiins.COM", None));
        assert!(
            pinned.allows("api.braiins.com", Some(8443)),
            "an entry without a port speaks for every port"
        );
        assert!(!pinned.allows("evil.com", None));
    }

    #[test]
    fn an_entry_with_a_port_restricts_to_it() {
        let pinned = policy(&["api.braiins.com:8443"]);

        assert!(pinned.allows("api.braiins.com", Some(8443)));
        assert!(!pinned.allows("api.braiins.com", Some(443)));
        assert!(!pinned.allows("api.braiins.com", None));
    }

    #[test]
    fn a_wildcard_takes_one_label_and_never_the_apex() {
        let pinned = policy(&["*.braiins.com"]);

        assert!(pinned.allows("api.braiins.com", None));
        assert!(
            !pinned.allows("braiins.com", None),
            "the apex is listed separately or not at all"
        );
        assert!(
            !pinned.allows("a.b.braiins.com", None),
            "one label only, as TLS and cookies do it"
        );
    }

    #[test]
    fn a_wildcard_ignores_case_on_both_sides() {
        assert!(
            policy(&["*.Braiins.com"]).allows("API.braiins.COM", None),
            "an operator writes these by hand; case must not silently void one"
        );
    }

    #[test]
    fn a_wildcard_cannot_be_escaped_by_a_lookalike_suffix() {
        let pinned = policy(&["*.braiins.com"]);

        assert!(
            !pinned.allows("api.notbraiins.com", None),
            "the label boundary must be a real dot, not a substring match"
        );
        assert!(!pinned.allows("braiins.com.evil.com", None));
    }

    #[test]
    fn a_cidr_range_admits_addresses_inside_it() {
        let lan = policy(&["10.0.0.0/8"]);

        assert!(lan.allows("10.1.2.3", None));
        assert!(!lan.allows("11.1.2.3", None));
        assert!(
            lan.allows("10.1.2.3", Some(4028)),
            "a range says nothing about ports"
        );
    }

    #[test]
    fn a_cidr_range_never_matches_a_hostname() {
        assert!(
            !policy(&["10.0.0.0/8"]).allows("rig.local", None),
            "resolving here would approve one address and let the fetch dial another"
        );
    }

    #[test]
    fn address_families_do_not_cross() {
        assert!(!policy(&["10.0.0.0/8"]).allows("fd00::1", None));
        assert!(!policy(&["fd00::/8"]).allows("10.1.2.3", None));
    }

    #[test]
    fn ipv6_ranges_compare_by_prefix() {
        let lan = policy(&["fd00::/8"]);

        assert!(lan.allows("fd00::1", None));
        assert!(lan.allows("fdff:ffff::1", None));
        assert!(!lan.allows("fe80::1", None));
    }

    #[test]
    fn a_zero_length_prefix_admits_the_whole_family() {
        assert!(policy(&["0.0.0.0/0"]).allows("203.0.113.9", None));
        assert!(!policy(&["0.0.0.0/0"]).allows("fd00::1", None));
    }

    #[test]
    fn a_malformed_entry_admits_nothing() {
        assert!(!policy(&["10.0.0.0/nonsense"]).allows("10.1.2.3", None));
        assert!(!policy(&["10.0.0.0/33"]).allows("10.1.2.3", None));
        assert!(!policy(&["not a host/8"]).allows("10.1.2.3", None));
    }

    #[test]
    fn the_pool_type_admits_its_api_and_nothing_else() {
        let egress = find("braiins-pool")
            .egress
            .expect("BUG: braiins-pool is egress-pinned");

        assert!(egress.allows("api.braiins.com", None));
        assert!(!egress.allows("braiins.com", None));
        assert!(!egress.allows("attacker.example", None));
    }

    fn find(id: &str) -> CredentialType {
        builtins()
            .into_iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("BUG: builtin {id:?} must exist"))
    }

    #[test]
    fn builtin_ids_are_unique() {
        let mut ids: Vec<_> = builtins().into_iter().map(|t| t.id).collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(count, ids.len(), "duplicate builtin credential type id");
    }

    /// A type without artwork falls back to a glyph meaning "some credential",
    /// so a new built-in that forgot one would render plausibly rather than fail.
    #[test]
    fn every_builtin_ships_artwork_that_reaches_the_wire_as_its_own_bytes() {
        use base64::Engine as _;

        for builtin in BuiltinType::ALL {
            let id = builtin.id();
            let icon = builtin
                .schema()
                .icon
                .unwrap_or_else(|| panic!("BUG: builtin {id:?} must ship its own artwork"));
            assert_eq!(icon.mime_type, "image/svg+xml", "for {id:?}");

            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&icon.data)
                .unwrap_or_else(|_| panic!("BUG: the encoded asset for {id:?} must decode"));
            let svg = String::from_utf8(decoded).expect("BUG: an SVG asset is UTF-8");
            assert!(
                svg.contains("<svg") && svg.contains("</svg>"),
                "the wire form for {id:?} must carry the asset itself, not a path to it"
            );
        }
    }

    #[test]
    fn braiins_pool_is_egress_pinned() {
        let egress = find("braiins-pool")
            .egress
            .expect("BUG: braiins-pool must be egress-pinned");
        assert!(egress.allow_hosts.iter().any(|h| h == "api.braiins.com"));
    }

    #[test]
    fn generics_are_not_egress_pinned() {
        assert!(find("generic-token").egress.is_none());
        assert!(find("generic-userpass").egress.is_none());
    }

    #[test]
    fn secret_field_uses_password_format() {
        let t = find("braiins-pool");
        let (_, token) = t.fields.first().expect("BUG: braiins-pool has a field");
        assert!(matches!(
            &token.kind,
            ParamKind::String {
                format: Some(StringFormat::Password),
                ..
            }
        ));
    }

    #[test]
    fn every_matchable_entry_form_passes_the_check() {
        for entry in [
            "api.example.com",
            "api.example.com:8443",
            "*.example.com",
            "*.example.com:8443",
            "10.0.0.0/8",
            "fd00::/8",
            "fd00::1",
            "[fd00::1]",
            "[fd00::1]:8443",
            "192.0.2.7",
        ] {
            assert!(check_entry(entry).is_ok(), "{entry:?} must validate");
        }
    }

    #[test]
    fn nonsense_entries_are_named_not_swallowed() {
        for entry in [
            "",
            "two words",
            "http://api.example.com",
            "10.0.0.0/99",
            "*.",
            "*..example.com",
            "*.*.example.com",
            "api.*.example.com",
            "*",
            ":8443",
            "[fd00::1",
            "fd00::1]",
            "[]",
            "[not-an-ip]",
            "[10.0.0.1]",
            "[fd00::1]x",
            "[fd00::1]:notaport",
            "[fd00::1]:99999",
        ] {
            assert!(check_entry(entry).is_err(), "{entry:?} must be rejected");
        }
    }

    #[test]
    fn bracketed_ipv6_entries_match_their_bare_authority() {
        // `authority_of` hands the host over unbracketed,
        // so the entry's brackets must not reach the comparison.
        assert!(policy(&["[fd00::1]"]).allows("fd00::1", None));
        assert!(policy(&["[fd00::1]"]).allows("fd00::1", Some(8443)));
        assert!(policy(&["[fd00::1]:8443"]).allows("fd00::1", Some(8443)));
        assert!(!policy(&["[fd00::1]:8443"]).allows("fd00::1", Some(9000)));
        assert!(!policy(&["[fd00::1]:8443"]).allows("fd00::1", None));
        assert!(!policy(&["[fd00::1]"]).allows("fd00::2", None));
    }

    #[test]
    fn ip_entries_compare_numerically_not_textually() {
        // The request host arrives canonicalised,
        // so the operator's spelling must not have to match it letter for letter.
        assert!(policy(&["fd00:0:0:0:0:0:0:1"]).allows("fd00::1", None));
        assert!(policy(&["[fd00:0:0:0:0:0:0:1]:8443"]).allows("fd00::1", Some(8443)));
    }

    #[test]
    fn a_port_that_does_not_parse_is_rejected_not_dropped() {
        // The matcher's split treats `host:notaport` as a bare host on any port,
        // so letting it through would save an entry meaning more than was written.
        assert!(check_entry("api.example.com:notaport").is_err());
        assert!(check_entry("api.example.com:99999").is_err());
    }
}

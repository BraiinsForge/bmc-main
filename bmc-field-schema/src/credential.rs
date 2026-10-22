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

//! Credential-type schema — a named set of secret fields, built on this crate's [`ScalarField`]
//! so one form renderer drives both params and credential fields.
//!
//! Here rather than in the firmware crate so `bmc-widget-codegen` can read each type's fields.

use std::net::{IpAddr, Ipv6Addr};
use std::sync::LazyLock;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{EnumControl, ParamKey, ScalarField, ScalarKind, StringFormat, StringParam};

/// A kind of account a widget can bind, e.g. a Braiins Pool API token.
/// `fields` are what the user configures; the variables a widget may embed
/// as `{{ credential.<slot>.<field_key> }}` come from [`Self::spendable_fields`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialType {
    /// Stable id, referenced by widget manifests and accounts.
    pub id: String,
    pub name: String,
    pub description: String,
    /// The configured fields; secret fields carry [`StringFormat::Password`].
    pub fields: IndexMap<ParamKey, ScalarField>,
    /// Absent means the secret may be sent anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<EgressPin>,
    /// Absent means the frontend renders its own generic glyph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
}

impl CredentialType {
    /// The fields a widget may spend through `{{ credential.<slot>.<field> }}`.
    ///
    /// Usually these are the configured fields,
    /// plus any the type derives from them, such as a username and password pair's `basic`.
    /// A local file token configures a `path` and spends the `token` read from it,
    /// so the path never becomes a placeholder.
    #[must_use]
    pub fn spendable_fields(&self) -> Vec<ParamKey> {
        let field_key = |name: &str| {
            ParamKey::try_new(name.to_owned()).expect("BUG: a field name is identifier-shaped")
        };
        match BuiltinType::from_id(&self.id) {
            Some(BuiltinType::LocalFileToken) => vec![field_key(FILE_TOKEN_FIELD)],
            Some(builtin) => self
                .fields
                .keys()
                .cloned()
                .chain(builtin.derived_field_names().iter().copied().map(field_key))
                .collect(),
            None => self.fields.keys().cloned().collect(),
        }
    }
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

/// Where a credential's secret may be sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressPin {
    /// Only to these hosts. Each entry is one of:
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
    /// An empty list admits nothing.
    Hosts(Vec<String>),
    /// Only to the host and port of the address in this configured field,
    /// the account's own service, such as a Home Assistant instance.
    /// An address that is missing or does not parse admits nothing.
    Address { field: String },
}

impl EgressPin {
    /// Whether this pin permits sending one account's secret to `host`,
    /// reading the account's configured fields through `configured`.
    ///
    /// `host` and `port` are the request's, as [`authority_of`] gives them.
    ///
    /// Comparison is ASCII-case-insensitive.
    /// A non-ASCII host is compared as-is,
    /// so an internationalised domain must be listed in punycode.
    #[must_use]
    pub fn allows_account<'a>(
        &self,
        host: &str,
        port: Option<u16>,
        configured: impl Fn(&str) -> Option<&'a str>,
    ) -> bool {
        match self {
            Self::Hosts(entries) => entries.iter().any(|entry| entry_allows(entry, host, port)),
            // Compared as parsed, never read back as an entry,
            // so the address cannot pick up the entry grammar's wildcards or ranges.
            Self::Address { field } => configured(field).and_then(service_authority).is_some_and(
                |(pinned_host, pinned_port)| {
                    pinned_host.eq_ignore_ascii_case(host) && pinned_port == port
                },
            ),
        }
    }

    /// [`Self::allows_account`] for an account with no fields configured.
    #[cfg(test)]
    fn allows(&self, host: &str, port: Option<u16>) -> bool {
        self.allows_account(host, port, |_| None)
    }
}

/// Host and effective port of a parsed URL.
///
/// The port is the one connected to, not the one spelled: an entry
/// spelling `api.example.com:443` matches `https://api.example.com/`.
///
/// Goes through the typed [`url::Host`] rather than `host_str`,
/// which keeps the brackets around an IPv6 literal.
/// Rendering the address instead gives the bare form a CIDR entry parses,
/// and a domain arrives IDNA-encoded, so an internationalised name
/// is already punycode by the time a pin sees it.
#[must_use]
pub fn authority_of(url: &url::Url) -> Option<(String, Option<u16>)> {
    let host = match url.host()? {
        url::Host::Domain(domain) => domain.to_owned(),
        url::Host::Ipv4(addr) => addr.to_string(),
        url::Host::Ipv6(addr) => addr.to_string(),
    };

    Some((host, url.port_or_known_default()))
}

fn service_authority(address: &str) -> Option<(String, Option<u16>)> {
    let url = url::Url::parse(address).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    authority_of(&url)
}

/// A service address as an account stores it: the bare origin,
/// such as `http://homeassistant.local:8123`.
///
/// Parsed with `url::Url`, the parser the egress pin reads requests with,
/// so the two agree on the host the pin admits.
///
/// # Errors
///
/// A capitalised English sentence naming what is wrong with the address,
/// fit to show beside the field.
pub fn service_origin(address: &str) -> Result<String, &'static str> {
    let url = url::Url::parse(address).map_err(|_| "This is not a valid address")?;
    // Widgets reach the service over a WebSocket, and the host has no `wss://` yet (BDK-1000).
    if url.scheme() == "https" {
        return Err(
            "Encrypted (https://) addresses are not supported yet. Enter the http:// address",
        );
    }
    if url.scheme() != "http" {
        return Err("The address must start with http://");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The address must not carry a user name or password");
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("Enter the address alone, without a path after it");
    }
    Ok(url.origin().ascii_serialization())
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
    LocalFileToken,
    HomeAssistant,
}

impl BuiltinType {
    /// In catalog order: [`builtins`] preserves it, and the picker follows.
    pub const ALL: [Self; 5] = [
        Self::GenericToken,
        Self::GenericUserpass,
        Self::BraiinsPool,
        Self::LocalFileToken,
        Self::HomeAssistant,
    ];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::GenericToken => "generic-token",
            Self::GenericUserpass => "generic-userpass",
            Self::BraiinsPool => "braiins-pool",
            Self::LocalFileToken => "local-file-token",
            Self::HomeAssistant => "home-assistant",
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
            Self::LocalFileToken => local_file_token(),
            Self::HomeAssistant => home_assistant(),
        }
    }

    #[must_use]
    pub fn egress(self) -> Option<EgressPin> {
        self.schema().egress
    }

    /// The fields [`Self::derived_fields`] computes, known without any values.
    #[must_use]
    pub const fn derived_field_names(self) -> &'static [&'static str] {
        match self {
            Self::GenericUserpass => &[USERPASS_BASIC_FIELD],
            Self::HomeAssistant => &[HOME_ASSISTANT_WEBSOCKET_URL_FIELD],
            Self::GenericToken | Self::BraiinsPool | Self::LocalFileToken => &[],
        }
    }

    /// The spendable fields this type computes from its configured ones,
    /// which `configured` reads by name.
    /// A field is left out when an input is missing,
    /// so a placeholder spending it is refused rather than sent half-built.
    ///
    /// The local file token's `token` is read from a file, not computed,
    /// and so is not among them.
    #[must_use]
    pub fn derived_fields<'a>(
        self,
        configured: impl Fn(&str) -> Option<&'a str>,
    ) -> Vec<(&'static str, String)> {
        match self {
            Self::GenericUserpass => match (
                configured(USERPASS_USERNAME_FIELD),
                configured(USERPASS_PASSWORD_FIELD),
            ) {
                (Some(username), Some(password)) => {
                    vec![(USERPASS_BASIC_FIELD, basic_credential(username, password))]
                }
                (None, _) | (_, None) => Vec::new(),
            },
            Self::HomeAssistant => configured(HOME_ASSISTANT_URL_FIELD)
                .and_then(websocket_url)
                .map(|url| vec![(HOME_ASSISTANT_WEBSOCKET_URL_FIELD, url)])
                .unwrap_or_default(),
            Self::GenericToken | Self::BraiinsPool | Self::LocalFileToken => Vec::new(),
        }
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
static LOCAL_FILE_TOKEN_ICON: LazyLock<Icon> =
    LazyLock::new(|| svg_icon(include_str!("../assets/local-file-token.svg")));
static HOME_ASSISTANT_ICON: LazyLock<Icon> =
    LazyLock::new(|| svg_icon(include_str!("../assets/home-assistant.svg")));

fn secret_field(name: &str, description: &str) -> ScalarField {
    string_field(name, description, Some(StringFormat::Password))
}

fn string_field(name: &str, description: &str, format: Option<StringFormat>) -> ScalarField {
    ScalarField {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        is_optional: false,
        kind: ScalarKind::String(StringParam {
            format,
            min_length: None,
            max_length: None,
            enum_values: Vec::new(),
            enum_control: EnumControl::Dropdown,
            default_value: None,
            placeholder: None,
        }),
    }
}

fn field_map<const N: usize>(entries: [(&str, ScalarField); N]) -> IndexMap<ParamKey, ScalarField> {
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

pub const USERPASS_USERNAME_FIELD: &str = "username";
pub const USERPASS_PASSWORD_FIELD: &str = "password";

/// The spendable field of a username and password pair
/// that carries both as one HTTP Basic credential.
pub const USERPASS_BASIC_FIELD: &str = "basic";

/// The credential an `Authorization: Basic` header carries:
/// `username:password` in base64, encoded as UTF-8 per RFC 7617.
#[must_use]
pub fn basic_credential(username: &str, password: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
}

fn generic_userpass() -> CredentialType {
    CredentialType {
        id: BuiltinType::GenericUserpass.id().to_owned(),
        name: "Username & password".to_owned(),
        description: "A username and password pair.".to_owned(),
        fields: field_map([
            (
                USERPASS_USERNAME_FIELD,
                string_field("Username", "The account username.", None),
            ),
            (
                USERPASS_PASSWORD_FIELD,
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
        egress: Some(EgressPin::Hosts(vec![BRAIINS_POOL_HOST.to_owned()])),
        icon: Some(BRAIINS_POOL_ICON.clone()),
    }
}

/// Where Boser publishes its local API token on a Braiins OS miner.
pub const LOCAL_BOS_TOKEN_PATH: &str = "/var/run/boser-api.token";

/// The configured field of a local file token: where to read it from.
pub const FILE_TOKEN_PATH_FIELD: &str = "path";

/// The spendable field of a local file token: what was read.
pub const FILE_TOKEN_FIELD: &str = "token";

fn local_file_token() -> CredentialType {
    CredentialType {
        id: BuiltinType::LocalFileToken.id().to_owned(),
        name: "Local file token".to_owned(),
        description: format!(
            "A token that a service on this device writes to a file.\n\n\
             Made for miners running Braiins OS. BMC reads the local BOS API token from \
             `{LOCAL_BOS_TOKEN_PATH}` and widgets use it to talk to the local miner. \
             A miner password change does not affect them.\n\n\
             Enter the file path only. The token is read on the device. It is never stored in \
             the account and never shown."
        ),
        fields: field_map([(
            FILE_TOKEN_PATH_FIELD,
            string_field(
                "Token file path",
                &format!(
                    "Absolute path to a file with the token on one line, for example \
                     {LOCAL_BOS_TOKEN_PATH}. The file may appear later; the widget authenticates \
                     once it exists. Keep it outside /etc and /var/log, which support archives \
                     collect."
                ),
                None,
            ),
        )]),
        egress: None,
        icon: Some(LOCAL_FILE_TOKEN_ICON.clone()),
    }
}

/// The configured field of a Home Assistant account: the instance's address.
pub const HOME_ASSISTANT_URL_FIELD: &str = "url";

/// The configured field of a Home Assistant account: a long-lived access token.
pub const HOME_ASSISTANT_TOKEN_FIELD: &str = "token";

/// The spendable field derived from the address: the instance's WebSocket API.
pub const HOME_ASSISTANT_WEBSOCKET_URL_FIELD: &str = "websocket_url";

/// Where a Home Assistant instance at `address` serves its WebSocket API:
/// `ws` for `http`, `wss` for `https`, at `/api/websocket`.
fn websocket_url(address: &str) -> Option<String> {
    let mut url = url::Url::parse(address).ok()?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => return None,
    };
    url.set_scheme(scheme).ok()?;
    url.set_path("/api/websocket");
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

fn home_assistant() -> CredentialType {
    let address = ScalarField {
        name: "Address".to_owned(),
        description: Some(
            "The address you open Home Assistant at, such as http://homeassistant.local:8123."
                .to_owned(),
        ),
        is_optional: false,
        kind: ScalarKind::String(StringParam {
            format: Some(StringFormat::Uri),
            min_length: None,
            max_length: None,
            enum_values: Vec::new(),
            enum_control: EnumControl::Dropdown,
            default_value: None,
            placeholder: Some("http://homeassistant.local:8123".to_owned()),
        }),
    };

    CredentialType {
        id: BuiltinType::HomeAssistant.id().to_owned(),
        name: "Home Assistant".to_owned(),
        description: "A Home Assistant instance and a long-lived access token for it. \
                      The token is only ever sent to that address."
            .to_owned(),
        fields: field_map([
            (HOME_ASSISTANT_URL_FIELD, address),
            (
                HOME_ASSISTANT_TOKEN_FIELD,
                secret_field(
                    "Long-lived access token",
                    "Create one in Home Assistant on your profile page, under Security.",
                ),
            ),
        ]),
        egress: Some(EgressPin::Address {
            field: HOME_ASSISTANT_URL_FIELD.to_owned(),
        }),
        icon: Some(HOME_ASSISTANT_ICON.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(entries: &[&str]) -> EgressPin {
        EgressPin::Hosts(entries.iter().map(|e| (*e).to_owned()).collect())
    }

    #[test]
    fn an_empty_list_admits_nothing() {
        assert!(!policy(&[]).allows("anywhere.example.com", None));
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
        assert_eq!(egress, EgressPin::Hosts(vec!["api.braiins.com".to_owned()]));
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
            ScalarKind::String(StringParam {
                format: Some(StringFormat::Password),
                ..
            })
        ));
    }

    /// The file path is what the operator configures; the token is what a
    /// widget may spend. Showing `PATH` as a placeholder would let a widget
    /// send the file name instead of its content.
    #[test]
    fn a_local_file_token_spends_its_token_not_its_path() {
        let t = find("local-file-token");
        let configured: Vec<&str> = t.fields.keys().map(ParamKey::as_str).collect();
        assert_eq!(configured, [FILE_TOKEN_PATH_FIELD]);
        let spendable: Vec<String> = t
            .spendable_fields()
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect();
        assert_eq!(spendable, [FILE_TOKEN_FIELD]);
    }

    #[test]
    fn a_username_and_password_pair_also_spends_its_basic_credential() {
        let spendable: Vec<String> = find("generic-userpass")
            .spendable_fields()
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect();

        assert_eq!(
            spendable,
            [
                USERPASS_USERNAME_FIELD,
                USERPASS_PASSWORD_FIELD,
                USERPASS_BASIC_FIELD
            ]
        );
    }

    #[test]
    fn basic_credential_matches_the_rfc_7617_examples() {
        assert_eq!(
            basic_credential("Aladdin", "open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(
            basic_credential("test", "123£"),
            "dGVzdDoxMjPCow==",
            "non-ASCII must encode as UTF-8"
        );
    }

    #[test]
    fn a_username_and_password_pair_derives_its_basic_credential() {
        let configured = |field: &str| match field {
            USERPASS_USERNAME_FIELD | USERPASS_PASSWORD_FIELD => Some("root"),
            _ => None,
        };

        assert_eq!(
            BuiltinType::GenericUserpass.derived_fields(configured),
            [(USERPASS_BASIC_FIELD, "cm9vdDpyb290".to_owned())]
        );
    }

    #[test]
    fn half_a_pair_derives_nothing() {
        let username_only = |field: &str| (field == USERPASS_USERNAME_FIELD).then_some("root");

        assert!(
            BuiltinType::GenericUserpass
                .derived_fields(username_only)
                .is_empty(),
            "a missing password must not encode as if it were empty"
        );
    }

    #[test]
    fn every_type_derives_exactly_the_fields_it_names() {
        // Address-shaped, so a field derived from an address has one to parse.
        let everything = |_: &str| Some("http://x.example");
        for builtin in BuiltinType::ALL {
            let derived: Vec<&str> = builtin
                .derived_fields(everything)
                .into_iter()
                .map(|(field, _)| field)
                .collect();
            assert_eq!(
                derived,
                builtin.derived_field_names(),
                "codegen offers a placeholder for every name, so {:?} must resolve each",
                builtin.id()
            );
        }
    }

    #[test]
    fn basic_credential_keeps_a_colon_in_the_password() {
        assert_eq!(basic_credential("root", "a:b"), "cm9vdDphOmI=");
    }

    #[test]
    fn every_other_type_spends_exactly_what_it_configures() {
        for builtin in BuiltinType::ALL {
            if matches!(
                builtin,
                BuiltinType::LocalFileToken
                    | BuiltinType::GenericUserpass
                    | BuiltinType::HomeAssistant
            ) {
                continue;
            }
            let t = builtin.schema();
            let configured: Vec<ParamKey> = t.fields.keys().cloned().collect();
            assert_eq!(t.spendable_fields(), configured, "for {:?}", builtin.id());
        }
    }

    #[test]
    fn the_local_file_token_is_not_egress_pinned_and_takes_a_plain_path() {
        let t = find("local-file-token");
        assert!(t.egress.is_none());
        let (_, path) = t.fields.first().expect("BUG: local-file-token has a field");
        assert!(matches!(
            &path.kind,
            ScalarKind::String(StringParam { format: None, .. })
        ));
    }

    #[test]
    fn the_type_description_names_the_shared_token_path() {
        let t = find("local-file-token");
        assert!(t.description.contains(LOCAL_BOS_TOKEN_PATH));
        let (_, path) = t.fields.first().expect("BUG: local-file-token has a field");
        let description = path.description.as_deref().unwrap_or_default();
        assert!(description.contains(LOCAL_BOS_TOKEN_PATH));
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

    fn derived_websocket_url(address: &str) -> Option<String> {
        BuiltinType::HomeAssistant
            .derived_fields(|field| (field == HOME_ASSISTANT_URL_FIELD).then_some(address))
            .into_iter()
            .find(|(field, _)| *field == HOME_ASSISTANT_WEBSOCKET_URL_FIELD)
            .map(|(_, url)| url)
    }

    #[test]
    fn a_home_assistant_address_derives_its_websocket_api() {
        for (address, expected) in [
            (
                "http://homeassistant.local:8123",
                "ws://homeassistant.local:8123/api/websocket",
            ),
            (
                "http://homeassistant.local:8123/",
                "ws://homeassistant.local:8123/api/websocket",
            ),
            ("http://192.168.1.10", "ws://192.168.1.10/api/websocket"),
            (
                "https://ha.example.com",
                "wss://ha.example.com/api/websocket",
            ),
            (
                "https://ha.example.com/",
                "wss://ha.example.com/api/websocket",
            ),
            (
                "https://ha.example.com:8443",
                "wss://ha.example.com:8443/api/websocket",
            ),
        ] {
            assert_eq!(
                derived_websocket_url(address).as_deref(),
                Some(expected),
                "from {address}"
            );
        }
    }

    #[test]
    fn an_address_that_does_not_parse_derives_no_websocket_api() {
        for address in ["", "homeassistant.local:8123", "ftp://ha.example.com"] {
            assert_eq!(derived_websocket_url(address), None, "from {address:?}");
        }
    }

    fn home_assistant_pin() -> EgressPin {
        BuiltinType::HomeAssistant
            .egress()
            .expect("BUG: home-assistant is egress-pinned")
    }

    fn admits(address: Option<&str>, host: &str, port: u16) -> bool {
        home_assistant_pin().allows_account(host, Some(port), |field| {
            (field == HOME_ASSISTANT_URL_FIELD)
                .then_some(address)
                .flatten()
        })
    }

    #[test]
    fn a_home_assistant_token_may_reach_its_address_and_nothing_else() {
        let address = Some("http://homeassistant.local:8123");

        assert!(admits(address, "homeassistant.local", 8123));
        assert!(
            !admits(address, "homeassistant.local", 80),
            "the pin holds the port as well as the host"
        );
        assert!(!admits(address, "attacker.example", 8123));
    }

    #[test]
    fn an_address_without_a_port_pins_its_schemes_default() {
        let address = Some("http://homeassistant.local");

        assert!(
            admits(address, "homeassistant.local", 80),
            "ws:// shares http's default port"
        );
        assert!(!admits(address, "homeassistant.local", 8123));
    }

    #[test]
    fn an_ipv6_address_pins_its_bare_host() {
        assert!(admits(Some("http://[fd00::1]:8123"), "fd00::1", 8123));
    }

    #[test]
    fn an_address_is_never_read_as_a_wildcard() {
        let address = Some("http://*.lan:8123");

        assert!(
            url::Url::parse("http://*.lan:8123").is_ok(),
            "the URL parser keeps `*` in a host, so only the comparison stops it",
        );
        assert!(!admits(address, "evil.lan", 8123));
        assert!(admits(address, "*.lan", 8123));
    }

    #[test]
    fn a_missing_or_broken_address_admits_nothing() {
        for address in [None, Some("not a url"), Some("ftp://homeassistant.local")] {
            assert!(
                !admits(address, "homeassistant.local", 8123),
                "{address:?} must pin nowhere, never everywhere"
            );
        }
        assert!(
            !home_assistant_pin().allows("homeassistant.local", Some(8123)),
            "a check that reads no fields must not take the empty list as no pin"
        );
    }

    #[test]
    fn a_service_address_is_stored_as_its_bare_origin() {
        for (address, stored) in [
            (
                "http://homeassistant.local:8123",
                "http://homeassistant.local:8123",
            ),
            (
                "http://HomeAssistant.local:8123/",
                "http://homeassistant.local:8123",
            ),
            ("http://ha.example.com:80", "http://ha.example.com"),
            ("http://[fd00::1]:8123", "http://[fd00::1]:8123"),
        ] {
            assert_eq!(service_origin(address), Ok(stored.to_owned()), "{address}");
        }
    }

    #[test]
    fn a_service_address_with_anything_beyond_its_origin_is_refused() {
        for address in [
            "",
            "not a url",
            "homeassistant.local:8123",
            "ftp://homeassistant.local",
            "https://homeassistant.local:8123",
            "http://user:secret@homeassistant.local:8123",
            "http://homeassistant.local:8123/lovelace",
            "http://homeassistant.local:8123/?a=1",
            "http://homeassistant.local:8123/#top",
        ] {
            assert!(
                service_origin(address).is_err(),
                "{address:?} must be refused"
            );
        }
    }
}

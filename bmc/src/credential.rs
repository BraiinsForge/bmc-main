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

//! Credential-type schema — a named set of secret fields a widget can be granted an account of,
//! reusing the shared [`bmc_field_schema`] field vocabulary so the same form renderer drives both.

use bmc_field_schema::{ParamDefinition, ParamKey, ParamKind, StringFormat};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A kind of account a widget can bind, e.g. a Braiins Pool API token.
/// Each field key is the interpolation variable a widget embeds as `{{ credential.<slot>.<field_key> }}`.
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
}

/// Hosts a credential type's secrets may be sent to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EgressPolicy {
    /// Matched exactly against the request's normalised authority — full host (every subdomain
    /// label) and port, lowercased and IDNA-normalised — never the path. Subdomain wildcards are
    /// not yet supported (open question).
    pub allow_hosts: Vec<String>,
}

/// The fixed set of firmware-provided credential types.
#[must_use]
pub fn builtins() -> Vec<CredentialType> {
    vec![generic_token(), generic_userpass(), braiins_pool()]
}

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
        id: "generic-token".to_owned(),
        name: "Token".to_owned(),
        description:
            "A single API token or bearer secret.\n\n**The widget may send them to any host.**"
                .to_owned(),
        fields: field_map([(
            "token",
            secret_field("Token", "The API token or bearer secret."),
        )]),
        egress: None,
    }
}

fn generic_userpass() -> CredentialType {
    CredentialType {
        id: "generic-userpass".to_owned(),
        name: "Username & password".to_owned(),
        description: "**The widget may send them to any host.**".to_owned(),
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
    }
}

fn braiins_pool() -> CredentialType {
    CredentialType {
        id: "braiins-pool".to_owned(),
        name: "Braiins Pool".to_owned(),
        description: "A Braiins Pool API token used to fetch your worker stats.".to_owned(),
        fields: field_map([(
            "token",
            secret_field("API token", "Your Braiins Pool API token."),
        )]),
        egress: Some(EgressPolicy {
            allow_hosts: vec!["api.braiins.com".to_owned()],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

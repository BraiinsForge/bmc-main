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

//! The host's credential placeholders, the one part of the client that knows them.
//!
//! The host substitutes `{{ credential.<slot>.<field> }}` in the address a widget dials
//! and in each message it sends, so the session writes these where the account goes.

/// The field `field` of the account bound to the credential slot `slot`.
pub(crate) fn placeholder(slot: &str, field: &str) -> String {
    let mut text = String::from("{{ credential.");
    text.push_str(slot);
    text.push('.');
    text.push_str(field);
    text.push_str(" }}");
    text
}

/// Whether the host would read `text` as naming a credential:
/// `{{`, then any whitespace, then `credential`, as its placeholder scan matches one.
/// Stricter than the scan, it also matches the escaped `{{{{` and an unclosed reference.
pub(crate) fn names_credential(text: &str) -> bool {
    text.match_indices("{{").any(|(at, open)| {
        text[at + open.len()..]
            .trim_start()
            .starts_with("credential")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn a_placeholder_names_the_slot_and_the_field() {
        assert_eq!(
            placeholder("home-assistant", "websocket_url"),
            "{{ credential.home-assistant.websocket_url }}"
        );
    }

    #[wasm_test]
    fn text_naming_a_credential_is_reserved() {
        for text in [
            "{{ credential.ha.token }}",
            "{{credential.other.secret}}",
            "before {{\u{a0}credential.ha.token }} after",
            "{{{{ credential.ha.token }}",
            "{{ credential.ha.token",
        ] {
            assert!(names_credential(text), "{text:?}");
        }
    }

    #[wasm_test]
    fn other_braces_are_not_reserved() {
        for text in [
            "{{ states('sensor.outdoor') }}",
            "{{ user }}",
            "{{",
            "a credential {{ x }}",
            "}} credential",
        ] {
            assert!(!names_credential(text), "{text:?}");
        }
    }
}

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

//! Copy owned by a crate with no formatter (a schema, a wire protocol),
//! worded later by whichever `Intl` knows the language.
//!
//! [`message_descriptor!`] takes the grammar of `format_message!`,
//! so `formatjs extract` collects it with the rest;
//! `bmc_intl::format_lazy` words it once a language is known.
//!
//! A lazy message carries no arguments, since values can only be checked where they exist
//! and a missing one is a formatting error. Text that needs values is a `format_message!`
//! where the values are, taking the lazy label as one of them.

/// A message as [`message_descriptor!`] declares it.
///
/// Neither `Display` nor convertible to a string. Without an `Intl` there is no language
/// to word it in, so it cannot become a `String` as English by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: &'static str,
    pub default_message: &'static str,
    pub description: Option<&'static str>,
}

/// Declare a message where its data is, in the grammar of `format_message!` without `values`.
/// `default_message` is the English ICU pattern, `description` the translator's context,
/// `id` a hash of both unless given. The pattern is parsed at compile time
/// and must take no arguments — or exactly the `arguments` named,
/// the form a formatter's macro uses for a pattern it has the values of.
#[macro_export]
macro_rules! message_descriptor {
    (
        $(id: $id:literal,)?
        default_message: $default_message:literal
        $(, description: $description:literal)?
        $(,)?
    ) => {{
        const _: () = $crate::__validate_message_values!($default_message;);
        $crate::message_descriptor!(@declare
            $(id: $id,)?
            default_message: $default_message
            $(, description: $description)?
        )
    }};
    (
        $(id: $id:literal,)?
        default_message: $default_message:literal
        $(, description: $description:literal)?
        , arguments: [$($argument:ident),* $(,)?]
        $(,)?
    ) => {{
        const _: () = $crate::__validate_message_values!($default_message; $($argument),*);
        $crate::message_descriptor!(@declare
            $(id: $id,)?
            default_message: $default_message
            $(, description: $description)?
        )
    }};
    (@declare $($descriptor:tt)*) => {{
        const DATA: (&str, &str, ::core::option::Option<&str>) =
            $crate::__message_descriptor!($($descriptor)*);
        $crate::Message {
            id: DATA.0,
            default_message: DATA.1,
            description: DATA.2,
        }
    }};
}

#[doc(hidden)]
pub use formatjs_intl_macros::{__message_descriptor, __validate_message_values};

#[cfg(test)]
mod tests {
    use super::Message;

    #[test]
    fn a_given_id_is_kept_and_the_description_is_carried() {
        let message = message_descriptor!(
            id: "probe.given",
            default_message: "Probe",
            description: "A test fixture",
        );
        assert_eq!(
            message,
            Message {
                id: "probe.given",
                default_message: "Probe",
                description: Some("A test fixture"),
            }
        );
    }

    #[test]
    fn the_id_hashes_the_english_and_the_description() {
        let bare = message_descriptor!(default_message: "Probe");
        let described =
            message_descriptor!(default_message: "Probe", description: "A test fixture");
        assert_eq!(bare.id.len(), 10, "the content hash is ten characters");
        assert_eq!(bare.description, None);
        assert_ne!(
            bare.id, described.id,
            "the description is part of what the id hashes"
        );
    }

    #[test]
    fn a_message_is_a_constant() {
        const TITLE: Message = message_descriptor!(default_message: "Probe");
        assert_eq!(TITLE.default_message, "Probe");
    }

    #[test]
    fn a_pattern_with_arguments_declares_them() {
        let counted = message_descriptor!(
            default_message: "{count, plural, one {{count} probe} other {{count} probes}}",
            arguments: [count],
        );
        assert_eq!(counted.id.len(), 10);
    }
}

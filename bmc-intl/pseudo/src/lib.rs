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

//! How the `en-XA` pseudo-locale respells English:
//! a message's literals accented and the whole bracketed, its arguments as they were.
//! The table is formatjs's own, read from the descriptor the samples also live in.

use std::sync::LazyLock;

use bmc_shared_utils::language::Language;
use formatjs_icu_messageformat::Options;
use formatjs_icu_messageformat_parser::MessageFormatElement;
use formatjs_icu_messageformat_parser::parser::Parser;
use formatjs_icu_messageformat_parser::printer::print_ast;
use serde::Deserialize;

/// The pseudo-locale this crate respells English for.
pub const LANGUAGE: Language = Language::EN_XA;

#[derive(Deserialize)]
struct Descriptor {
    brackets: [String; 2],
    letters: String,
    doubled: String,
}

static DESCRIPTOR: LazyLock<Descriptor> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../assets/intl/pseudo-locale.json"))
        .expect("BUG: the pseudo-locale descriptor parses")
});

static LETTERS: LazyLock<Vec<char>> = LazyLock::new(|| {
    let letters: Vec<char> = DESCRIPTOR.letters.chars().collect();
    assert_eq!(
        letters.len(),
        52,
        "BUG: one letter per ASCII capital, then small"
    );
    letters
});

/// Every letter the pseudo-locale spells English with, which the device's faces must draw.
#[must_use]
pub fn letters() -> &'static str {
    &DESCRIPTOR.letters
}

/// `text` with each ASCII letter from the accent table and the doubled vowels twice,
/// for words no message carries, such as CLDR's relative times.
#[must_use]
pub fn accent(text: &str) -> String {
    let mut spelled = String::with_capacity(text.len() * 2);
    for character in text.chars() {
        let index = match character {
            'A'..='Z' => usize::from(character as u8 - b'A'),
            'a'..='z' => 26 + usize::from(character as u8 - b'a'),
            _ => {
                spelled.push(character);
                continue;
            }
        };
        let letter = LETTERS[index];
        spelled.push(letter);
        if DESCRIPTOR.doubled.contains(character) {
            spelled.push(letter);
        }
    }
    spelled
}

fn accent_literals(elements: &mut [MessageFormatElement]) {
    for element in elements {
        match element {
            MessageFormatElement::Literal(literal) => literal.value = accent(&literal.value),
            MessageFormatElement::Plural(plural) => {
                for option in plural.options.values_mut() {
                    accent_literals(&mut option.value);
                }
            }
            MessageFormatElement::Select(select) => {
                for option in select.options.values_mut() {
                    accent_literals(&mut option.value);
                }
            }
            MessageFormatElement::Tag(tag) => accent_literals(&mut tag.children),
            MessageFormatElement::Argument(_)
            | MessageFormatElement::Number(_)
            | MessageFormatElement::Date(_)
            | MessageFormatElement::Time(_)
            | MessageFormatElement::Pound(_) => {}
        }
    }
}

/// `source`, an English pattern, as the pseudo-locale speaks it, still a pattern.
/// One that does not parse is handed back, for formatting to refuse as it would have.
#[must_use]
pub fn respell(source: &str) -> String {
    let Ok(mut elements) = Parser::new(source, Options::default_parser_options()).parse() else {
        return source.to_owned();
    };
    accent_literals(&mut elements);
    let [open, close] = &DESCRIPTOR.brackets;
    elements.insert(0, MessageFormatElement::literal(open.clone()));
    elements.push(MessageFormatElement::literal(close.clone()));
    print_ast(&elements)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accenting_leaves_everything_but_ascii_letters() {
        assert_eq!(accent("5 h ago, Ẇī-"), "5 ħ ȧȧɠǿǿ, Ẇī-");
        assert_eq!(accent(""), "");
    }

    #[test]
    fn a_respelled_message_keeps_its_arguments_and_branches() {
        let respelled = respell("{count, plural, one {{count} widget} other {{count} widgets}}");
        Parser::new(&respelled, Options::default_parser_options())
            .parse()
            .expect("BUG: a respelled message is still a pattern");
        for kept in ["plural", "one", "other", "{count}"] {
            assert!(respelled.contains(kept), "{kept} is not in {respelled}");
        }
    }

    #[test]
    fn a_source_that_is_not_a_pattern_is_handed_back() {
        assert_eq!(respell("{unclosed"), "{unclosed");
    }
}

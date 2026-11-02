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

use std::ops::Range;
use std::sync::LazyLock;

use formatjs_icu_messageformat::Options;
use formatjs_icu_messageformat_parser::MessageFormatElement;
use formatjs_icu_messageformat_parser::parser::{Parser, ParserOptions};
use serde::Deserialize;

/// Where the pseudo-locale's pack lives, relative to the repository root.
pub const PACK_ROOT: &str = "assets/intl/pseudo";

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

/// `character` from the accent table, a doubled vowel twice; anything but an ASCII letter as it is.
fn push_accented(character: char, spelled: &mut String) {
    let index = match character {
        'A'..='Z' => usize::from(character as u8 - b'A'),
        'a'..='z' => 26 + usize::from(character as u8 - b'a'),
        _ => {
            spelled.push(character);
            return;
        }
    };
    let letter = LETTERS[index];
    spelled.push(letter);
    if DESCRIPTOR.doubled.contains(character) {
        spelled.push(letter);
    }
}

/// Where each literal of `source` lies, in characters, a branch's
/// and a tag's included. None when `source` does not parse.
fn literal_spans(source: &str) -> Option<Vec<Range<usize>>> {
    let options = ParserOptions {
        capture_location: true,
        ..Options::default_parser_options()
    };
    let elements = Parser::new(source, options).parse().ok()?;
    let mut spans = Vec::new();
    collect_literal_spans(&elements, &mut spans);
    Some(spans)
}

fn collect_literal_spans(elements: &[MessageFormatElement], spans: &mut Vec<Range<usize>>) {
    for element in elements {
        match element {
            MessageFormatElement::Literal(literal) => {
                let location = literal
                    .location
                    .as_ref()
                    .expect("BUG: a parse that captures locations locates every literal");
                spans.push(location.start.offset..location.end.offset);
            }
            MessageFormatElement::Plural(plural) => {
                for option in plural.options.values() {
                    collect_literal_spans(&option.value, spans);
                }
            }
            MessageFormatElement::Select(select) => {
                for option in select.options.values() {
                    collect_literal_spans(&option.value, spans);
                }
            }
            MessageFormatElement::Tag(tag) => collect_literal_spans(&tag.children, spans),
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
///
/// Its literals' letters are accented where they stand, the rest kept as written:
/// printing the parsed message back would lean on formatjs's printer,
/// which escapes a lone `'` between two arguments so that the second is read as text.
#[must_use]
pub fn respell(source: &str) -> String {
    let Some(spans) = literal_spans(source) else {
        return source.to_owned();
    };
    let [open, close] = &DESCRIPTOR.brackets;
    let mut spoken = String::with_capacity(open.len() + source.len() * 2 + close.len());
    spoken.push_str(open);
    for (index, character) in source.chars().enumerate() {
        if spans.iter().any(|span| span.contains(&index)) {
            push_accented(character, &mut spoken);
        } else {
            spoken.push(character);
        }
    }
    spoken.push_str(close);
    spoken
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respelling_accents_nothing_but_ascii_letters() {
        assert_eq!(respell("5 h ago, Ẇī-"), "[5 ħ ȧȧɠǿǿ, Ẇī-]");
        assert_eq!(respell(""), "[]");
    }

    #[test]
    fn every_branch_of_a_plural_is_respelled_and_its_syntax_kept() {
        assert_eq!(
            respell("{count, plural, one {{count} widget} other {{count} widgets}}"),
            "[{count, plural, one {{count} ẇīḓɠḗḗŧ} other {{count} ẇīḓɠḗḗŧş}}]"
        );
    }

    #[test]
    fn every_branch_of_a_select_is_respelled_and_its_syntax_kept() {
        assert_eq!(
            respell("{who, select, me {You} other {Someone}} joined"),
            "[{who, select, me {Ẏǿǿŭŭ} other {Şǿǿḿḗḗǿǿƞḗḗ}} ĵǿǿīƞḗḗḓ]"
        );
    }

    #[test]
    fn a_quote_between_arguments_stays_a_literal_quote() {
        assert_eq!(respell("{n}''{x}"), "[{n}''{x}]");
    }

    #[test]
    fn a_source_that_is_not_a_pattern_is_handed_back() {
        assert_eq!(respell("{unclosed"), "{unclosed");
    }

    #[test]
    fn letters_after_wide_characters_are_respelled_where_they_stand() {
        assert_eq!(
            respell("Čas {time} – <b>ňak</b> ok"),
            "[Čȧȧş {time} – <b>ňȧȧķ</b> ǿǿķ]",
            "spans counted in bytes would land the accents off these letters"
        );
    }
}

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

//! What a translation must be beside its English: a pattern the runtime parses,
//! with the same arguments of the same types, and no locale-grouped `#`.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

use formatjs_icu_messageformat::Options;
use formatjs_icu_messageformat_parser::types::Type;
use formatjs_icu_messageformat_parser::{
    ArgumentKind, MessageFormatElement, Parser, StructuralComparisonError, is_structurally_same,
    message_arguments,
};

use crate::translator::{Entries, Extraction};

/// The problems of each translated entry of `entries` against the English `extracted` holds,
/// in id order; an untranslated entry, or one the extraction lacks, is the stale check's.
#[must_use]
pub fn translations(extracted: &Extraction, entries: &Entries) -> Vec<String> {
    entries
        .iter()
        .filter_map(|(id, entry)| {
            Some((
                id,
                entry.translation.as_deref()?,
                &extracted.get(id)?.message,
            ))
        })
        .filter_map(|(id, translation, english)| translation_problem(id, english, translation))
        .collect()
}

fn translation_problem(id: &str, english: &str, translation: &str) -> Option<String> {
    let parse = |pattern: &str| Parser::new(pattern, Options::default_parser_options()).parse();
    let translated = match parse(translation) {
        Ok(ast) => ast,
        Err(error) => return Some(format!("{id}: {error}")),
    };
    if has_pound(&translated) {
        return Some(format!(
            "{id}: `#` in a plural branch is locale-grouped; write the argument by name"
        ));
    }
    let source = match parse(english) {
        Ok(ast) => ast,
        Err(error) => return Some(format!("{id}: the English does not parse: {error}")),
    };
    match structurally_same(&source, &translated, id) {
        Ok(()) => None,
        Err(Difference::Structure(difference)) => Some(format!(
            "{id}: {}",
            describe(&difference, &source, &translated)
        )),
        Err(Difference::Panic(text)) => Some(format!("{id}: {text}")),
    }
}

/// The difference in the translator's terms: which side lacks what.
fn describe(
    difference: &StructuralComparisonError,
    source: &[MessageFormatElement],
    translated: &[MessageFormatElement],
) -> String {
    match difference {
        StructuralComparisonError::MissingVariable {
            variable, var_type, ..
        } if message_arguments(translated).contains_key(variable) => format!(
            "{} is {} in the English but not in the translation",
            placeholder(variable, *var_type == Type::Tag),
            kind_name(*var_type)
        ),
        StructuralComparisonError::MissingVariable { .. }
        | StructuralComparisonError::DifferentVariableCount { .. } => {
            renamed(&message_arguments(source), &message_arguments(translated))
        }
    }
}

fn renamed(
    english: &BTreeMap<String, BTreeSet<ArgumentKind>>,
    translation: &BTreeMap<String, BTreeSet<ArgumentKind>>,
) -> String {
    let only_in = |side: &BTreeMap<String, BTreeSet<ArgumentKind>>,
                   other: &BTreeMap<String, BTreeSet<ArgumentKind>>| {
        side.iter()
            .filter(|(name, _)| !other.contains_key(*name))
            .map(|(name, kinds)| placeholder(name, kinds.contains(&ArgumentKind::Tag)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (only_in(english, translation), only_in(translation, english)) {
        (dropped, added) if added.is_empty() => format!("the translation drops {dropped}"),
        (dropped, added) if dropped.is_empty() => {
            format!("the translation adds {added}, which the English does not have")
        }
        (dropped, added) => format!("the translation has {added} where the English has {dropped}"),
    }
}

/// `name` as a message writes it: `<b>` for a tag, `{count}` for an argument.
fn placeholder(name: &str, is_tag: bool) -> String {
    if is_tag {
        format!("<{name}>")
    } else {
        format!("{{{name}}}")
    }
}

fn kind_name(kind: Type) -> &'static str {
    match kind {
        Type::Literal => "text",
        Type::Argument => "a plain argument",
        Type::Number => "a number",
        Type::Date => "a date",
        Type::Time => "a time",
        Type::Select => "a select",
        Type::Plural => "a plural",
        Type::Pound => "a `#`",
        Type::Tag => "a tag",
    }
}

/// Whether a plural branch prints its count with `#`.
/// `#` formats the count for the locale ("1,234"), while `{count}` prints it as passed.
/// Outside a plural, `#` parses as plain text, so it never shows up here.
fn has_pound(elements: &[MessageFormatElement]) -> bool {
    elements.iter().any(|element| match element {
        MessageFormatElement::Pound(_) => true,
        MessageFormatElement::Plural(plural) => plural
            .options
            .values()
            .any(|option| has_pound(&option.value)),
        MessageFormatElement::Select(select) => select
            .options
            .values()
            .any(|option| has_pound(&option.value)),
        MessageFormatElement::Tag(tag) => has_pound(&tag.children),
        MessageFormatElement::Literal(_)
        | MessageFormatElement::Argument(_)
        | MessageFormatElement::Number(_)
        | MessageFormatElement::Date(_)
        | MessageFormatElement::Time(_) => false,
    })
}

thread_local! {
    static COMPARING: Cell<bool> = const { Cell::new(false) };
}

enum Difference {
    Structure(StructuralComparisonError),
    /// The parser's own text: its panic carries nothing typed to phrase.
    Panic(String),
}

/// `is_structurally_same`, with the panic it raises on a variable used
/// with two types in one message taken as the difference it is.
fn structurally_same(
    source: &[MessageFormatElement],
    translated: &[MessageFormatElement],
    id: &str,
) -> Result<(), Difference> {
    static QUIET_WHILE_COMPARING: Once = Once::new();
    QUIET_WHILE_COMPARING.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !COMPARING.get() {
                previous(info);
            }
        }));
    });
    COMPARING.set(true);
    let compared = catch_unwind(AssertUnwindSafe(|| {
        is_structurally_same(source, translated, id.to_owned())
    }));
    COMPARING.set(false);
    match compared {
        Ok(Ok(())) => Ok(()),
        Ok(Err(difference)) => Err(Difference::Structure(difference)),
        Err(panic) => Err(Difference::Panic(
            panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "a variable used with two types".to_owned()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translator::{Entry, Extracted};

    fn problems(cases: &[(&str, &str, Option<&str>)]) -> Vec<String> {
        let (extracted, entries) = cases
            .iter()
            .map(|(id, english, translation)| {
                (
                    ((*id).to_owned(), Extracted::new(english)),
                    ((*id).to_owned(), Entry::new(english, *translation)),
                )
            })
            .unzip::<_, _, Extraction, Entries>();
        translations(&extracted, &entries)
    }

    #[test]
    fn a_translation_that_does_not_parse_hides_none_of_the_files_other_problems() {
        let english = "{n, plural, one {{n} day} other {{n} days}}";
        let found = problems(&[
            ("no-other", english, Some("{n, plural, one {{n} 天}}")),
            ("pound", english, Some("{n, plural, other {# 天}}")),
        ]);
        let ids: Vec<&str> = found
            .iter()
            .filter_map(|problem| problem.split_once(':').map(|(id, _)| id))
            .collect();
        assert_eq!(ids, ["no-other", "pound"], "{found:?}");
    }

    #[test]
    fn a_hash_in_a_plural_branch_is_reported_and_a_named_argument_is_not() {
        let found = problems(&[
            (
                "grouped",
                "{n, plural, one {{n} day} other {{n} days}}",
                Some("{n, plural, =0 {无} other {# 天}}"),
            ),
            (
                "ordinal",
                "{n, selectordinal, one {{n}st} other {{n}th}}",
                Some("{n, selectordinal, other {#.}}"),
            ),
            (
                "named",
                "{n, plural, one {{n} day} other {{n} days}}",
                Some("{n, plural, other {{n} 天}}"),
            ),
            ("plain_hash", "Item #{n}", Some("第 #{n} 项")),
            (
                "after_the_plural",
                "{n, plural, other {{n} days}} #",
                Some("{n, plural, other {{n} 天}} #"),
            ),
            (
                "quoted",
                "{n, plural, other {'#' {n}}}",
                Some("{n, plural, other {'#' {n}}}"),
            ),
            (
                "nested_select",
                "{n, plural, other {{g, select, other {#}}}}",
                Some("{n, plural, other {{g, select, other {#}}}}"),
            ),
            ("untranslated", "{n, plural, other {{n} days}}", None),
        ]);
        assert_eq!(
            found,
            [
                "grouped: `#` in a plural branch is locale-grouped; write the argument by name",
                "ordinal: `#` in a plural branch is locale-grouped; write the argument by name",
            ]
        );
    }

    #[test]
    fn a_translation_that_does_not_parse_is_named() {
        let found = problems(&[("broken", "Hello {name}", Some("你好 {name"))]);
        assert!(
            matches!(&found[..], [one] if one.starts_with("broken: ")),
            "{found:?}"
        );
    }

    #[test]
    fn a_translation_of_english_that_does_not_parse_is_named() {
        let found = problems(&[("broken", "Hello {name", Some("你好 {name}"))]);
        assert!(
            matches!(&found[..], [one] if one.starts_with("broken: the English does not parse: ")),
            "{found:?}"
        );
    }

    #[test]
    fn a_translation_with_other_arguments_or_types_is_named() {
        let found = problems(&[
            ("added", "Hello", Some("你好 {name}")),
            (
                "dropped",
                "Hello {name}, <b>{count}</b>",
                Some("你好 {count}"),
            ),
            ("renamed", "Hello {name}", Some("你好 {nom}")),
            ("retyped", "{count, number} items", Some("{count} 项")),
            ("same", "Hello {name}", Some("你好 {name}")),
            ("tag_retyped", "<b>Hi</b>", Some("{b} 你好")),
        ]);
        assert_eq!(
            found,
            [
                "added: the translation adds {name}, which the English does not have",
                "dropped: the translation drops <b>, {name}",
                "renamed: the translation has {nom} where the English has {name}",
                "retyped: {count} is a number in the English but not in the translation",
                "tag_retyped: <b> is a tag in the English but not in the translation",
            ]
        );
    }

    #[test]
    fn a_variable_used_with_two_types_is_a_problem_not_a_crash() {
        let found = problems(&[("mixed", "{x, number} {x}", Some("{x, number} {x, date}"))]);
        assert_eq!(found.len(), 1, "{found:?}");
    }
}

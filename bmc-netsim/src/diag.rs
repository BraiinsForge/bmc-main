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

//! A code frame for a blueprint load failure.
//!
//! `json5` stamps every parse error — a syntax slip, or a value rejected by a
//! validating type like [`crate::http_status::HttpStatus`] — with the source
//! line and column, which this turns into a caret-under-the-token frame.

use std::io::{self, IsTerminal};
use std::ops::Range;
use std::path::Path;

use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::SimpleFile;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
use codespan_reporting::term::{self, Config};

/// Print a `json5` load error to stderr as a code frame.
///
/// Colour is gated on stderr being a terminal, which termcolor's `Auto` does not
/// test for itself; `Auto` then drops colour for `TERM=dumb` or `NO_COLOR`.
pub fn emit_error(path: &Path, source: &str, err: &json5::Error) {
    if !io::stderr().is_terminal() {
        eprintln!("{}", render_error(path, source, err));
        return;
    }
    let json5::Error::Message { msg, location } = err;
    let Some(location) = location else {
        eprintln!("{}", bare(path, msg));
        return;
    };
    let writer = StandardStream::stderr(ColorChoice::Auto);
    let file = SimpleFile::new(path.display().to_string(), source);
    let rendered = term::emit_to_write_style(
        &mut writer.lock(),
        &config(),
        &file,
        &frame(source, msg, location.line, location.column),
    );
    // An error reporter that fails is worse than one that renders plainly.
    if rendered.is_err() {
        eprintln!("{}", render_error(path, source, err));
    }
}

/// Render the same frame to a string.
#[must_use]
pub fn render_error(path: &Path, source: &str, err: &json5::Error) -> String {
    let json5::Error::Message { msg, location } = err;
    let Some(location) = location else {
        return bare(path, msg);
    };
    let file = SimpleFile::new(path.display().to_string(), source);
    term::emit_into_string(
        &config(),
        &file,
        &frame(source, msg, location.line, location.column),
    )
    .unwrap_or_else(|_| bare(path, msg))
}

fn frame(source: &str, msg: &str, line: usize, column: usize) -> Diagnostic<()> {
    let diagnostic = Diagnostic::error()
        .with_message(summarize(msg))
        .with_label(Label::primary((), token_span(source, line, column)));
    match did_you_mean(msg) {
        Some(hint) => diagnostic.with_notes(vec![hint]),
        None => diagnostic,
    }
}

/// `json5` reports a syntax slip by handing back pest's `Display`, which already
/// carries its own caret frame; keep just the trailing `expected …` summary so
/// the frame is not drawn around a frame.
fn summarize(msg: &str) -> &str {
    msg.lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("= "))
        .unwrap_or(msg)
}

/// How far a key may stray and still read as a typo of a real one. `stauts` is
/// two edits from `status`; past this the nearest match is a different word.
const TYPO_DISTANCE: usize = 3;

/// Suggest the closest valid key for serde's unknown-field error, which reads
/// ``unknown field `x`, expected one of `a`, `b``` — a shape only serde emits,
/// so a message that does not match simply yields no hint.
fn did_you_mean(msg: &str) -> Option<String> {
    let (found, rest) = msg.strip_prefix("unknown field `")?.split_once('`')?;
    let nearest = rest
        .split_once("expected one of ")?
        .1
        .split(',')
        .map(|candidate| candidate.trim().trim_matches('`'))
        .filter(|candidate| !candidate.is_empty())
        .min_by_key(|candidate| strsim::levenshtein(found, candidate))?;
    (strsim::levenshtein(found, nearest) <= TYPO_DISTANCE)
        .then(|| format!("did you mean `{nearest}`?"))
}

/// Show a couple of lines either side of the offending token: a blueprint entry
/// reads as a block, and one line alone rarely says which device it belongs to.
fn config() -> Config {
    Config {
        before_label_lines: 2,
        after_label_lines: 2,
        ..Config::default()
    }
}

fn bare(path: &Path, msg: &str) -> String {
    format!("error: {msg}\n  --> {}", path.display())
}

/// Byte range of the token at 1-based `line`/`column`, so the label underlines
/// the whole offending literal rather than its first character.
fn token_span(source: &str, line: usize, column: usize) -> Range<usize> {
    let start = byte_offset(source, line, column);
    let len: usize = source
        .char_indices()
        .skip_while(|(index, _)| *index < start)
        .map(|(_, ch)| ch)
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+' | '_'))
        .map(char::len_utf8)
        .sum();
    start..(start + len.max(1)).min(source.len())
}

/// Byte index of a 1-based line/column pair, counting columns in characters as
/// `json5` reports them.
fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    let (mut at_line, mut at_column) = (1_usize, 1_usize);
    for (index, ch) in source.char_indices() {
        if at_line == line && at_column == column {
            return index;
        }
        if ch == '\n' {
            at_line += 1;
            at_column = 1;
        } else {
            at_column += 1;
        }
    }
    source.len()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::render_error;
    use crate::http_status::HttpStatus;

    #[test]
    fn frames_a_rejected_status_at_its_token() {
        let source = "{\n  status: 99,\n}";
        let err = json5::from_str::<BTreeMap<String, HttpStatus>>(source)
            .expect_err("BUG: 99 must be rejected");
        let frame = render_error(Path::new("fleet.json5"), source, &err);

        assert!(frame.contains("fleet.json5:2:11"), "located: {frame}");
        assert!(
            frame.contains("status: 99"),
            "shows the source line: {frame}"
        );
        assert!(
            frame.contains("^^"),
            "carets span the two-digit token: {frame}"
        );
    }

    #[test]
    fn frames_a_syntax_error_without_nesting_pests_own_frame() {
        let source = "{ status: }";
        let err = json5::from_str::<BTreeMap<String, HttpStatus>>(source)
            .expect_err("BUG: malformed json5");
        let frame = render_error(Path::new("fleet.json5"), source, &err);

        assert!(frame.starts_with("error: expected"), "summarized: {frame}");
        assert_eq!(
            frame.matches("─┐").count() + frame.matches("-->").count(),
            0,
            "pest's own frame must not survive inside ours:\n{frame}"
        );
    }

    #[test]
    fn suggests_the_key_a_typo_was_reaching_for() {
        assert_eq!(
            super::did_you_mean("unknown field `stauts`, expected one of `power_w`, `status`"),
            Some("did you mean `status`?".to_owned())
        );
    }

    #[test]
    fn offers_no_suggestion_for_a_key_unlike_any_of_them() {
        assert_eq!(
            super::did_you_mean("unknown field `elephant`, expected one of `power_w`, `status`"),
            None
        );
    }

    #[test]
    fn leaves_other_errors_unhinted() {
        assert_eq!(super::did_you_mean("99 is not a registered status"), None);
    }

    #[test]
    fn a_multibyte_character_earlier_on_the_line_does_not_shift_the_span() {
        // `json5` reports columns counted in characters, so the offset has to be
        // walked the same way — counting bytes would drag the span left.
        // Exercised against `token_span` directly: json5 0.4.1 panics parsing a
        // string that opens with a multi-byte character, so the loader cannot
        // reach this case through a quoted value.
        let source = "/* °° */ status: 99,";
        let column = source.chars().position(|c| c == '9').expect("BUG: has a 9") + 1;
        let span = super::token_span(source, 1, column);
        assert_eq!(
            source.get(span),
            Some("99"),
            "the span must cover the whole number"
        );
    }

    #[test]
    fn spans_the_whole_token_not_just_its_first_character() {
        let source = "{\n  status: 1234,\n}";
        let err = json5::from_str::<BTreeMap<String, HttpStatus>>(source)
            .expect_err("BUG: 1234 must be rejected");
        let frame = render_error(Path::new("fleet.json5"), source, &err);
        assert!(
            frame.contains("^^^^"),
            "four carets for four digits: {frame}"
        );
    }
}

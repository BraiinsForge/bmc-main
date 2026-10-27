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

//! How the intl and language-pack tools report, as `deck`'s console does:
//! results to stdout, an error and the output explaining it together on stderr.

use std::borrow::Cow;
use std::fmt::Display;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use console::{StyledObject, Term, strip_ansi_codes};
use tabled::Table;
use tabled::builder::Builder;
use tabled::settings::object::{Rows, Segment};
use tabled::settings::peaker::Priority;
use tabled::settings::style::{BorderColor, HorizontalLine};
use tabled::settings::themes::Colorization;
use tabled::settings::{Color, Style, Width};

/// Whether the last line written was blank, so blank lines never stack.
static AFTER_BLANK: AtomicBool = AtomicBool::new(false);

/// Whether stdout's reader has stopped reading, as `head` does.
static STDOUT_GONE: AtomicBool = AtomicBool::new(false);

/// Styled whichever stream it ends up on;
/// [`out`] and [`err_line`] strip the styling where that stream takes no colour.
fn style<D>(value: D) -> StyledObject<D> {
    console::style(value).force_styling(true)
}

fn for_stream(text: &str, colour: bool) -> Cow<'_, str> {
    if colour {
        Cow::Borrowed(text)
    } else {
        strip_ansi_codes(text)
    }
}

/// `text` and a newline on stdout; nothing once its reader is gone,
/// so the run still ends with its own exit code.
fn out(text: &str) {
    if STDOUT_GONE.load(Ordering::Relaxed) {
        return;
    }
    let text = for_stream(text, console::colors_enabled());
    if let Err(error) = writeln!(std::io::stdout().lock(), "{text}") {
        if error.kind() == ErrorKind::BrokenPipe {
            STDOUT_GONE.store(true, Ordering::Relaxed);
            return;
        }
        eprintln!("stdout: {error}");
        std::process::exit(1);
    }
}

/// A line of output, as it is, such as a dump.
pub fn line(text: impl Display) {
    out(&text.to_string());
    AFTER_BLANK.store(false, Ordering::Relaxed);
}

/// A line on stderr.
fn err_line(text: impl Display) {
    eprintln!(
        "{}",
        for_stream(&text.to_string(), console::colors_enabled_stderr())
    );
    AFTER_BLANK.store(false, Ordering::Relaxed);
}

/// An empty line, to set output apart; none right after another.
fn blank() {
    if !AFTER_BLANK.swap(true, Ordering::Relaxed) {
        out("");
    }
}

/// A tool's run, set apart by blank lines; its error is reported and exits 1.
pub fn run(run: impl FnOnce() -> Result<ExitCode, String>) -> ExitCode {
    blank();
    let code = run().unwrap_or_else(|failed| {
        error(&failed);
        ExitCode::FAILURE
    });
    blank();
    code
}

/// A success.
pub fn ok(message: &str) {
    line(format_args!("{} {message}", style("✓").green().bright()));
}

/// A failure; what explains it follows through [`details`].
pub fn error(message: &str) {
    err_line(format_args!("{} {message}", style("✗").red().bold()));
}

/// A literal value (a path, a message, a commit) inside a message.
#[must_use]
pub fn lit(value: &str) -> String {
    style(value).magenta().to_string()
}

/// `value` dimmed, for what is there to look up rather than to read.
#[must_use]
pub fn dim(value: &str) -> String {
    style(value).dim().to_string()
}

/// A value the run was given and refuses, quoted as clap shows one.
#[must_use]
pub fn refused(value: &str) -> String {
    style(format!("'{value}'")).yellow().bold().to_string()
}

/// A command-line option, quoted as clap shows one.
#[must_use]
pub fn option(name: &str) -> String {
    style(format!("'{name}'"))
        .cyan()
        .bright()
        .bold()
        .to_string()
}

/// A path on disk, underlined as a terminal shows a link to a file.
#[must_use]
pub fn path(path: &Path) -> String {
    style(path.display()).blue().underlined().to_string()
}

/// What went wrong with the file at `path`.
#[must_use]
pub fn at(path: &Path, error: impl Display) -> String {
    format!("{}: {error}", self::path(path))
}

/// `missing`, which the dev shell provides: `formatjs is not on PATH`.
#[must_use]
pub fn outside_dev_shell(missing: &str) -> String {
    format!(
        "{missing}; enter the dev shell ({})",
        command("nix develop")
    )
}

/// Why `tool` would not start; not being on PATH means the dev shell is not entered.
#[must_use]
pub fn spawn_failed(tool: &str, error: &std::io::Error) -> String {
    if error.kind() == ErrorKind::NotFound {
        outside_dev_shell(&format!("{tool} is not on PATH"))
    } else {
        format!("{tool}: {error}")
    }
}

/// The name a line is about, such as a surface's.
#[must_use]
pub fn name(value: &str) -> String {
    style(value).bold().to_string()
}

/// A section's title, set off by a blank line and a rule.
pub fn header(title: &str) {
    blank();
    line(format_args!(
        "{} {} {}",
        style("──").cyan(),
        style(title).bold(),
        style("─".repeat(40)).cyan()
    ));
    blank();
}

/// What explains an error, on its stream: `heading`,
/// then each of `lines` indented under it.
pub fn details(heading: &str, lines: &[String]) {
    err_line(format_args!(" {heading}"));
    for detail in lines {
        err_line(format_args!("   {detail}"));
    }
}

/// A command to run, set apart in colour on a shaded background;
/// in backticks where stderr takes no colour.
#[must_use]
pub fn command(text: &str) -> String {
    if console::colors_enabled_stderr() {
        style(format!(" {text} "))
            .green()
            .on_color256(236)
            .to_string()
    } else {
        format!("`{text}`")
    }
}

/// The widest a table grows however wide the terminal,
/// so a row still reads as one line.
const MAX_TABLE_WIDTH: usize = 120;

/// `rows` in aligned columns under a dim header and a rule,
/// every other row shaded; cells may be styled.
///
/// Wider than the terminal or [`MAX_TABLE_WIDTH`],
/// cells wrap at word boundaries.
pub fn table(columns: &[&str], rows: &[Vec<String>]) {
    let terminal = Term::stdout()
        .size_checked()
        .map(|(_, columns)| usize::from(columns));
    let width = terminal.map_or(MAX_TABLE_WIDTH, |width| width.min(MAX_TABLE_WIDTH));
    line(framed(columns, rows, width));
}

fn framed(columns: &[&str], rows: &[Vec<String>], width: usize) -> Table {
    let mut builder = Builder::default();
    builder.push_record(columns.iter().map(|column| dim(column)));

    for row in rows {
        builder.push_record(row.iter().cloned());
    }

    let mut table = builder.build();

    // No vertical lines: a terminal's line spacing breaks them between rows.
    table.with(Style::blank().horizontals([(1, HorizontalLine::new('─').intersection('─'))]));
    table.modify(Segment::all(), BorderColor::filled(Color::DIMMED));

    // Every other row a step above the terminal's black.
    let shade = Color::rgb_bg(38, 38, 38);

    // Row 0 is the header; the shading starts at the second row under it,
    // and takes in the gaps between its cells so the row reads unbroken.
    for index in (2..=rows.len()).step_by(2) {
        table
            .with(Colorization::exact([shade.clone()], Rows::one(index)))
            .modify(
                Rows::one(index),
                BorderColor::new().left(shade.clone()).right(shade.clone()),
            );
    }
    table.with(
        Width::wrap(width)
            .keep_words(true)
            .priority(Priority::max(true)),
    );

    table
}

#[cfg(test)]
mod tests {
    use console::measure_text_width;

    use super::*;

    #[test]
    fn a_table_aligns_its_columns_whatever_the_cells_hold() {
        let text = console::strip_ansi_codes(
            &framed(
                &["language", "English", "id"],
                &[vec!["zh".to_owned(), lit("Loading [1/2]"), dim("bbb")]],
                MAX_TABLE_WIDTH,
            )
            .to_string(),
        )
        .into_owned();
        let line = |needle: &str| {
            text.lines()
                .find(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("`{needle}` is drawn:\n{text}"))
        };
        let (header, row) = (line("English"), line("bbb"));
        assert_eq!(
            header.find("English"),
            row.find("Loading [1/2]"),
            "brackets stay text:\n{text}"
        );
        assert_eq!(
            header.find("id"),
            row.find("bbb"),
            "under its header:\n{text}"
        );
        let widths: Vec<usize> = text.lines().map(measure_text_width).collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "every line as wide:\n{text}"
        );
    }

    #[test]
    fn a_table_wider_than_the_terminal_wraps_its_cells_at_words() {
        let long =
            "The device did not come back online after the upgrade, it may still be restarting";
        let table = framed(
            &["language", "English", "id"],
            &[vec!["zh".to_owned(), lit(long), dim("bbb")]],
            40,
        );
        let text = console::strip_ansi_codes(&table.to_string()).into_owned();
        assert!(
            text.lines().all(|line| measure_text_width(line) <= 40),
            "within the terminal:\n{text}"
        );
        assert!(
            text.contains("restarting"),
            "every word kept whole:\n{text}"
        );
        let first = text
            .lines()
            .find(|line| line.contains("zh"))
            .unwrap_or_default();
        assert!(
            first.contains("bbb"),
            "the narrow columns keep their cells; the widest one wraps:\n{text}"
        );
    }
}

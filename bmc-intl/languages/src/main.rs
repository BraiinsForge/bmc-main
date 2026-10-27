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

//! Language packs on the author's computer, before any reaches a device:
//! checked with the loader the device runs, so the report is what the device would log.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bmc_intl_console as console;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Language packs: check them as a device reads them, compile their flag")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report what a device makes of the packs under ROOTS, laid over the roots it reads;
    /// exit 1 unless the load takes everything.
    Check {
        /// Directories holding one `<lang>/` directory per pack.
        roots: Vec<PathBuf>,
    },
    /// Compile a pack's `flag.svg` into the `flag.bin` the device draws.
    CompileFlag { svg: PathBuf, bin: PathBuf },
}

fn main() -> ExitCode {
    let command = Args::parse().command;
    console::run(|| match command {
        Command::Check { roots } => check(&roots),
        Command::CompileFlag { svg, bin } => compile_flag(&svg, &bin),
    })
}

/// # Errors
/// A root that is no directory: checking it would check nothing.
fn check(roots: &[PathBuf]) -> Result<ExitCode, String> {
    if let Some(missing) = roots.iter().find(|root| !root.is_dir()) {
        return Err(format!("{}: no directory of packs", console::path(missing)));
    }
    let (translations, report) =
        bmc_intl::Translations::load(&[bmc_intl::packs::roots(), roots.to_vec()].concat());
    let taken: Vec<String> = translations
        .languages()
        .iter()
        .map(|language| console::lit(&language.tag()))
        .collect();
    console::ok(&format!("languages: {}", taken.join(", ")));
    if report.is_clean() {
        return Ok(ExitCode::SUCCESS);
    }
    let refused: Vec<String> = report
        .refused
        .iter()
        .map(|(language, reason)| format!("{}: {reason}", console::name(&language.tag())))
        .collect();
    let left_out = refused.len() + report.skipped.len() + report.invalid.len();
    console::error(&format!("{} left out", console::lit(&left_out.to_string())));
    for (heading, lines) in [
        ("refused", &refused),
        ("skipped", &report.skipped),
        ("invalid", &report.invalid),
    ] {
        if !lines.is_empty() {
            console::details(heading, lines);
        }
    }
    Ok(ExitCode::FAILURE)
}

fn compile_flag(svg: &Path, bin: &Path) -> Result<ExitCode, String> {
    let source = std::fs::read_to_string(svg).map_err(|error| console::at(svg, error))?;
    std::fs::write(bin, bmc_svg_compiler::compile_svg(&source))
        .map_err(|error| console::at(bin, error))?;
    Ok(ExitCode::SUCCESS)
}

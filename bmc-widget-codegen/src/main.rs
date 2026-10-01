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

//! Read a widget `manifest.json` and emit a typed-accessor `manifest_params.rs`.
//!
//! ```text
//! bmc-widget-codegen --manifest <path> --out <path>
//! ```
//!
//! [`bmc_widget_codegen::generate`] returns source pre-formatted by
//! `prettyplease`; this driver writes that to disk and then runs `rustfmt` over it
//! so the committed artifact matches the workspace's canonical style. Failure is
//! non-fatal — the file is still written in `prettyplease`'s canonical form.

use std::ffi::OsStr;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result};
use bmc_widget_codegen::NameCollisions;
use bmc_widget_manifest::Manifest;
use clap::Parser;
use owo_colors::OwoColorize as _;

#[derive(Parser, Debug)]
#[command(name = "bmc-widget-codegen", about, version)]
struct Cli {
    /// Path to the widget's `manifest.json`.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,

    /// Path to write the generated Rust file. The parent directory must already exist.
    /// If the manifest declares no params, no file is written and the process exits 0.
    #[arg(long, value_name = "PATH")]
    out: PathBuf,

    /// Skip the `rustfmt` post-process. Use when this binary is invoked from
    /// inside the nix build sandbox where the formatter is unreachable.
    #[arg(long)]
    skip_format: bool,
}

fn main() -> ExitCode {
    let Err(err) = run() else {
        return ExitCode::SUCCESS;
    };
    let color = use_color();
    let message = match err.downcast_ref::<NameCollisions>() {
        Some(collisions) => collisions.render(color),
        None => format!("{err:?}"),
    };
    let label = if color {
        "error:".red().bold().to_string()
    } else {
        "error:".to_owned()
    };
    eprintln!("{label} {message}");
    ExitCode::FAILURE
}

/// The switch `common.justfile` exports: `FORCE_COLOR=0` is plain,
/// and any other non-empty value keeps color through a pipe.
/// A non-empty `NO_COLOR` outranks a forced color, as it does for rich;
/// with neither set, stderr must be a terminal.
fn use_color() -> bool {
    color_enabled(
        std::env::var_os("FORCE_COLOR").as_deref(),
        std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        std::io::stderr().is_terminal(),
    )
}

fn color_enabled(force_color: Option<&OsStr>, no_color: bool, stderr_is_terminal: bool) -> bool {
    match force_color {
        Some(force) if force == "0" => false,
        _ if no_color => false,
        Some(force) => !force.is_empty(),
        None => stderr_is_terminal,
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let body = std::fs::read_to_string(&cli.manifest)
        .with_context(|| format!("read manifest at {}", cli.manifest.display()))?;
    let manifest = <Manifest as std::str::FromStr>::from_str(&body)
        .with_context(|| format!("parse manifest at {}", cli.manifest.display()))?;

    if manifest.params.is_empty() && manifest.credentials.is_empty() {
        // Remove a stale file if present, so the drift-guard doesn't bark on a widget that had its
        // params or credentials dropped from the manifest.
        if cli.out.exists() {
            std::fs::remove_file(&cli.out)
                .with_context(|| format!("remove stale {}", cli.out.display()))?;
            eprintln!(
                "removed stale {} (manifest declares no params or credentials)",
                cli.out.display()
            );
        } else {
            eprintln!("manifest declares no params or credentials — nothing to emit");
        }
        return Ok(());
    }

    let relpath = relative_manifest_path(&cli.manifest, &cli.out);
    let formatted = bmc_widget_codegen::generate(&manifest, &relpath)?;

    std::fs::write(&cli.out, &formatted).with_context(|| format!("write {}", cli.out.display()))?;
    eprintln!("wrote {}", cli.out.display());

    if !cli.skip_format {
        run_rustfmt(&cli.out);
    }
    Ok(())
}

/// Best-effort `rustfmt` so the committed artifact matches the project's canonical
/// style; it finds the workspace `rustfmt.toml` by walking up from `path`, and
/// `--edition` mirrors the project formatter so the result matches CI's check.
/// A missing `rustfmt` is non-fatal — the file stays in `prettyplease` form.
///
/// Not `nix fmt`: the generated file is excluded from it, to keep the license
/// stamper off a regenerated artifact, which makes that pass a silent no-op.
fn run_rustfmt(path: &Path) {
    let result = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(path)
        .status();
    match result {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("rustfmt exited with {status} — generated file left unformatted"),
        Err(e) => eprintln!("rustfmt unavailable ({e}) — generated file left unformatted"),
    }
}

/// Compute the manifest path relative to the output file's directory, falling
/// back to the manifest's display string when no common ancestor exists.
/// Recorded in the generated file's header comment.
fn relative_manifest_path(manifest: &Path, out: &Path) -> String {
    let out_dir = out.parent().unwrap_or(Path::new("."));
    pathdiff(manifest, out_dir).map_or_else(
        || manifest.display().to_string(),
        |p| p.display().to_string(),
    )
}

/// Minimal relative-path computation via `canonicalize`. Returns `None` if
/// either side fails to resolve (e.g. the output file doesn't exist yet —
/// expected on first emit).
fn pathdiff(target: &Path, from_dir: &Path) -> Option<PathBuf> {
    let target = target.canonicalize().ok()?;
    let from_dir = from_dir.canonicalize().ok()?;
    let mut rel = PathBuf::new();
    let mut t = target.components().peekable();
    let mut f = from_dir.components().peekable();
    while t.peek().is_some() && f.peek().is_some() && t.peek() == f.peek() {
        t.next();
        f.next();
    }
    for _ in f {
        rel.push("..");
    }
    for c in t {
        rel.push(c.as_os_str());
    }
    Some(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agents_force_color_zero_is_plain_even_on_a_terminal() {
        assert!(!color_enabled(Some(OsStr::new("0")), false, true));
    }

    #[test]
    fn force_color_keeps_color_through_a_pipe() {
        assert!(color_enabled(Some(OsStr::new("1")), false, false));
    }

    #[test]
    fn no_color_outranks_a_forced_color() {
        assert!(!color_enabled(Some(OsStr::new("1")), true, true));
    }

    #[test]
    fn an_empty_force_color_is_plain() {
        assert!(!color_enabled(Some(OsStr::new("")), false, true));
    }

    #[test]
    fn unforced_color_follows_the_terminal() {
        assert!(color_enabled(None, false, true), "a terminal is colored");
        assert!(!color_enabled(None, false, false), "a pipe is plain");
        assert!(
            !color_enabled(None, true, true),
            "NO_COLOR strips a terminal"
        );
    }
}

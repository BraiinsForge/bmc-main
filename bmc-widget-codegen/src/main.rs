// Copyright (C) 2026  Braiins Systems s.r.o.

//! Read a widget `manifest.json` and emit a typed-accessor `manifest_params.rs`.
//!
//! ```text
//! bmc-widget-codegen --manifest <path> --out <path>
//! ```
//!
//! [`bmc_widget_codegen::generate`] returns source pre-formatted by
//! `prettyplease`; this driver writes that to disk and then invokes the
//! project's `nix fmt` against the output path so the committed artifact
//! matches the workspace's canonical style. Failure of `nix fmt` is
//! non-fatal — the file is still written in `prettyplease`'s canonical form.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result};
use bmc_widget_manifest::Manifest;
use clap::Parser;

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

    /// Skip the `nix fmt` post-process. Use when this binary is invoked from
    /// inside the nix build sandbox where the formatter is unreachable.
    #[arg(long)]
    skip_format: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let body = std::fs::read_to_string(&cli.manifest)
        .with_context(|| format!("read manifest at {}", cli.manifest.display()))?;
    let manifest = <Manifest as std::str::FromStr>::from_str(&body)
        .with_context(|| format!("parse manifest at {}", cli.manifest.display()))?;

    if manifest.params.is_empty() {
        // Plan's "empty params → no file emitted" rule. Remove a stale file if present
        // so the drift-guard doesn't bark on widgets that had params dropped from the manifest.
        if cli.out.exists() {
            std::fs::remove_file(&cli.out)
                .with_context(|| format!("remove stale {}", cli.out.display()))?;
            eprintln!(
                "removed stale {} (manifest has no params)",
                cli.out.display()
            );
        } else {
            eprintln!("manifest has no params — nothing to emit");
        }
        return Ok(());
    }

    let relpath = relative_manifest_path(&cli.manifest, &cli.out);
    let formatted = bmc_widget_codegen::generate(&manifest, &relpath)?;

    std::fs::write(&cli.out, &formatted).with_context(|| format!("write {}", cli.out.display()))?;
    eprintln!("wrote {}", cli.out.display());

    if !cli.skip_format {
        run_nix_fmt(&cli.out);
    }
    Ok(())
}

/// Best-effort `nix fmt <path>` so the committed artifact matches the project's
/// canonical Rust style (workspace `rustfmt.toml`, comment wrapping rules, etc.).
/// A missing `nix` or formatter is non-fatal — the file is still on disk in
/// `prettyplease`'s canonical form.
fn run_nix_fmt(path: &Path) {
    let result = Command::new("nix").args(["fmt", "--"]).arg(path).status();
    match result {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("nix fmt exited with {status} — generated file left unformatted"),
        Err(e) => eprintln!("nix fmt unavailable ({e}) — generated file left unformatted"),
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

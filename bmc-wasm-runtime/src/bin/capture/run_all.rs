// Copyright (C) 2026  Braiins Systems s.r.o.

//! `run-all` subcommand — build and capture all (or one) widget examples.
//!
//! Replaces `tools/capture_run.py`. Discovers examples, builds their WASM via
//! a single workspace `cargo build`, then runs `capture run` for each
//! size×variant combination.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use owo_colors::OwoColorize;

use bmc_wasm_runtime::capture_config;

// ── Constants ───────────────────────────────────────────────────────

const WASM_TARGET: &str = "wasm32-unknown-unknown";

use bmc_wasm_runtime::capture_config::CAPTURE_SIZES;

/// Alignment column for right-side timings.
const COL_WIDTH: usize = 50;

// ── Public interface ────────────────────────────────────────────────

pub struct RunAllArgs {
    pub example: Option<String>,
    /// Directory containing pre-built `.wasm` files. When set, cargo build is
    /// skipped and files are expected as `<name>.wasm` (underscored) in this
    /// directory. When `None`, the workspace is built locally.
    pub wasm_dir: Option<PathBuf>,
    /// Root directory containing widget subdirectories (each with `capture/`).
    /// Defaults to `examples`.
    pub widgets_dir: Option<PathBuf>,
    /// Root output directory for captures. Layout: `<output_dir>/<name>/current/<size>/`.
    /// Defaults to `captures`.
    pub output_dir: Option<PathBuf>,
    /// Capture widgets in parallel. `Some(0)` = nproc/2 threads,
    /// `Some(n)` = n threads, `None` = sequential.
    pub parallel: Option<usize>,
}

pub fn execute(args: &RunAllArgs) -> Result<()> {
    let widgets_dir = args
        .widgets_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("examples"));
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("captures"));

    if !widgets_dir.is_dir() {
        bail!(
            "'{}' directory not found — run from bmc-wasm-runtime/ or pass --widgets-dir",
            widgets_dir.display()
        );
    }

    let examples = match &args.example {
        Some(name) => {
            let dir = widgets_dir.join(name);
            if !dir.join("Cargo.toml").exists() {
                let available = discover_examples(&widgets_dir)?;
                bail!(
                    "example '{name}' not found.\nAvailable: {}",
                    available.join(", ")
                );
            }
            vec![name.clone()]
        }
        None => discover_examples(&widgets_dir)?,
    };

    let capture_binary =
        std::env::current_exe().context("failed to resolve capture binary path")?;

    // ── Resolve WASM directory ───────────────────────────────────────
    let wasm_dir = if let Some(dir) = &args.wasm_dir {
        section("Build (prebuilt)");
        for example in &examples {
            let wasm = wasm_in_dir(dir, example);
            if !wasm.exists() {
                eprintln!("  {} {}", "✗".red(), example.red());
                bail!("--wasm-dir: expected .wasm not found at {}", wasm.display());
            }
            eprintln!("  {} {}", "✓".green(), example.green());
        }
        eprintln!();
        dir.clone()
    } else {
        section("Build");
        let build_t0 = Instant::now();

        build_wasm_workspace(&widgets_dir, &examples)?;

        for example in &examples {
            eprintln!("  {} {}", "✓".green(), example.green());
        }

        let build_elapsed = build_t0.elapsed().as_secs_f64();
        eprintln!(
            "  {}",
            format!("compiled in {}", format_time(build_elapsed)).dimmed()
        );
        eprintln!();
        default_wasm_dir(&widgets_dir)
    };

    // ── Capture ─────────────────────────────────────────────────────
    section("Capture");
    let capture_t0 = Instant::now();

    if let Some(n) = args.parallel {
        if examples.len() > 1 {
            #[expect(clippy::integer_division)]
            let threads = if n == 0 {
                std::thread::available_parallelism().map_or(2, |p| (p.get() / 2).max(1))
            } else {
                n
            };
            capture_parallel(
                &capture_binary,
                &wasm_dir,
                &widgets_dir,
                &output_dir,
                &examples,
                threads,
            )?;
        } else {
            capture_sequential(
                &capture_binary,
                &wasm_dir,
                &widgets_dir,
                &output_dir,
                &examples,
            )?;
        }
    } else {
        capture_sequential(
            &capture_binary,
            &wasm_dir,
            &widgets_dir,
            &output_dir,
            &examples,
        )?;
    }

    let capture_elapsed = capture_t0.elapsed().as_secs_f64();
    eprintln!(
        "  {}",
        format!("captured in {}", format_time(capture_elapsed)).dimmed()
    );

    Ok(())
}

// ── Example discovery ───────────────────────────────────────────────

/// Discover all example widgets under the given directory.
pub fn discover_examples(widgets_dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(widgets_dir)
        .with_context(|| format!("failed to read {} directory", widgets_dir.display()))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() && path.join("Cargo.toml").exists() {
                entry.file_name().to_str().map(String::from)
            } else {
                None
            }
        })
        .collect();
    names.sort();
    Ok(names)
}

// ── WASM paths ──────────────────────────────────────────────────────

/// Default output directory when building locally.
fn default_wasm_dir(widgets_dir: &Path) -> PathBuf {
    widgets_dir.join("target").join(WASM_TARGET).join("release")
}

/// Resolve the `.wasm` path for an example within a given directory.
fn wasm_in_dir(dir: &Path, example: &str) -> PathBuf {
    let name = example.replace('-', "_");
    dir.join(format!("{name}.wasm"))
}

/// Example capture directory (where `config.toml` and fixtures live).
fn capture_dir(widgets_dir: &Path, example: &str) -> PathBuf {
    widgets_dir.join(example).join("capture")
}

// ── WASM building ───────────────────────────────────────────────────

/// Build one or all examples from the examples/ workspace.
///
/// Uses a single `cargo build` from the workspace root — cargo handles
/// parallelism internally, so we don't need to spawn threads.
fn build_wasm_workspace(widgets_dir: &Path, examples: &[String]) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--release", "--target", WASM_TARGET])
        .current_dir(widgets_dir);

    if examples.len() == 1 {
        cmd.args(["-p", &examples[0]]);
    } else {
        cmd.arg("--workspace");
    }

    let output = cmd
        .output()
        .context("failed to spawn cargo build for examples workspace")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cargo build failed for examples workspace:\n{stderr}");
    }
    Ok(())
}

// ── Per-example capture ─────────────────────────────────────────────

/// Capture a single widget (all sizes × variants). Returns elapsed seconds.
fn capture_widget(
    binary: &Path,
    wasm_dir: &Path,
    widgets_dir: &Path,
    output_dir: &Path,
    example: &str,
    show_progress: bool,
) -> Result<f64> {
    let wasm = wasm_in_dir(wasm_dir, example);
    let cap_dir = capture_dir(widgets_dir, example);
    let output_root = output_dir.join(example).join("current");

    // Wipe previous captures so stale frames don't linger.
    if output_root.exists() {
        let _ = std::fs::remove_dir_all(&output_root);
    }

    let fixture_sizes = configured_sizes(&cap_dir);
    if fixture_sizes.is_empty() {
        return Ok(0.0);
    }

    let variants = list_variants(binary, &wasm, &cap_dir);
    let t0 = Instant::now();

    for variant in &variants {
        for &(size_name, w, h) in CAPTURE_SIZES {
            let dimensions = format!("{w}x{h}");
            if !fixture_sizes.contains(&size_name.to_owned()) {
                continue;
            }

            let output = if variant == "_default" {
                output_root.join(size_name)
            } else {
                output_root.join(variant).join(size_name)
            };

            let size_label = if variant == "_default" {
                size_name.to_owned()
            } else {
                format!("{variant}/{size_name}")
            };

            if show_progress {
                progress(&format!("  {example} {size_label}..."));
            }

            let mut cmd = Command::new(binary);
            cmd.args(["run"])
                .arg(&wasm)
                .arg(format!("--size={dimensions}"))
                .arg(format!("--output={}", output.display()))
                .arg(format!("--capture-dir={}", cap_dir.display()));
            if variant != "_default" {
                cmd.arg(format!("--variant={variant}"));
            }

            let result = cmd
                .output()
                .with_context(|| format!("failed to spawn capture for {example} {size_label}"))?;

            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                bail!(
                    "capture failed for {example} {size_label}\n{}",
                    stderr.trim()
                );
            }
        }
    }

    Ok(t0.elapsed().as_secs_f64())
}

fn capture_sequential(
    binary: &Path,
    wasm_dir: &Path,
    widgets_dir: &Path,
    output_dir: &Path,
    examples: &[String],
) -> Result<()> {
    let is_tty = std::io::stderr().is_terminal();
    for example in examples {
        let elapsed = capture_widget(binary, wasm_dir, widgets_dir, output_dir, example, is_tty)?;
        clear_progress();
        widget_status_line(example, elapsed);
    }
    Ok(())
}

fn capture_parallel(
    binary: &Path,
    wasm_dir: &Path,
    widgets_dir: &Path,
    output_dir: &Path,
    examples: &[String],
    threads: usize,
) -> Result<()> {
    use rayon::prelude::*;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .context("failed to create thread pool")?;

    let results: Vec<_> = pool.install(|| {
        examples
            .par_iter()
            .map(|example| {
                let result =
                    capture_widget(binary, wasm_dir, widgets_dir, output_dir, example, false);
                (example.as_str(), result)
            })
            .collect()
    });

    let mut first_error = None;
    for (name, result) in &results {
        match result {
            Ok(elapsed) => widget_status_line(name, *elapsed),
            Err(e) => {
                eprintln!("  {} {} {e:#}", "✗".red(), name.red());
                if first_error.is_none() {
                    first_error = Some(format!("capture failed for {name}"));
                }
            }
        }
    }

    if let Some(msg) = first_error {
        bail!("{msg}");
    }
    Ok(())
}

// ── Config helpers ──────────────────────────────────────────────────

/// Read [fixtures] keys from capture/config.toml to determine which sizes have fixtures.
fn configured_sizes(capture_dir: &Path) -> Vec<String> {
    let Ok(config) = capture_config::load_from_capture_dir(capture_dir) else {
        return Vec::new();
    };
    config.fixtures.keys().cloned().collect()
}

/// Query available KV variants via the capture binary.
fn list_variants(binary: &Path, wasm: &Path, capture_dir: &Path) -> Vec<String> {
    let output = Command::new(binary)
        .args(["run"])
        .arg(wasm)
        .arg("--list-variants")
        .arg(format!("--capture-dir={}", capture_dir.display()))
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .collect()
        }
        _ => vec!["_default".to_owned()],
    }
}

// ── Display helpers ─────────────────────────────────────────────────

fn widget_status_line(name: &str, elapsed: f64) {
    let time_str = format_time(elapsed);
    let label = format!("  {} {}", "✓".green(), name.green());
    let visible_len = 2 + 2 + name.len() + 1;
    let dots = COL_WIDTH
        .saturating_sub(visible_len)
        .saturating_sub(time_str.len())
        .max(1);
    eprintln!(
        "{label} {}{} {time_str}",
        "·".repeat(dots).dimmed(),
        "".dimmed()
    );
}

fn section(title: &str) {
    let pad = COL_WIDTH.saturating_sub(title.len() + 1);
    eprintln!("{} {}", title.bold(), "─".repeat(pad).dimmed());
}

fn progress(msg: &str) {
    if std::io::stderr().is_terminal() {
        eprint!("\r\x1b[K{}", msg.dimmed());
        let _ = std::io::stderr().flush();
    }
}

fn clear_progress() {
    if std::io::stderr().is_terminal() {
        eprint!("\r\x1b[K");
        let _ = std::io::stderr().flush();
    }
}

#[expect(clippy::cast_sign_loss, clippy::modulo_arithmetic)]
fn format_time(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        let minutes = (seconds / 60.0).floor() as u32;
        let secs = seconds % 60.0;
        format!("{minutes}m {secs:.0}s")
    }
}

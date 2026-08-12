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

//! `run-all` subcommand — build and capture every widget across the given
//! cargo workspaces.
//!
//! Discovers widgets (immediate `Cargo.toml`-bearing subdirs of each workspace),
//! builds their WASM (one `cargo build --workspace` per workspace, or consumes
//! pre-built `.wasm` files per workspace), then runs `capture run`
//! for every configured (dataset, target) pair.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use owo_colors::OwoColorize;

use bmc_wasm_runtime::capture_config;

use super::run::StackProfiling;

// ── Constants ───────────────────────────────────────────────────────

const WASM_TARGET: &str = "wasm32-unknown-unknown";

use bmc_wasm_runtime::capture_config::CaptureConfig;

/// Alignment column for right-side timings.
const COL_WIDTH: usize = 50;

// ── Public interface ────────────────────────────────────────────────

pub struct RunAllArgs {
    pub widget: Option<String>,
    /// Cargo workspaces hosting widget crates.
    /// Each must contain immediate subdirs with `Cargo.toml`
    /// (one per widget). Non-empty.
    pub workspaces: Vec<PathBuf>,
    /// Pre-built `.wasm` directories paired positionally with `workspaces`.
    /// Length must be 0 (build everything from source) or equal to `workspaces.len()`
    /// (skip cargo build and read `<wasm_dir>/<widget>.wasm` plus `<wasm_dir>/assets`).
    pub wasm_dirs: Vec<PathBuf>,
    /// Root output directory for captures. Layout: `<output_dir>/<widget>/current/<size>/`.
    /// Widget names are globally unique across workspaces (enforced at discovery time),
    /// so no per-workspace namespacing is needed.
    pub output_dir: PathBuf,
    /// Capture widgets in parallel. `Some(0)` = nproc/2 threads,
    /// `Some(n)` = n threads, `None` = sequential.
    pub parallel: Option<usize>,
    pub stack_profiling: StackProfiling,
}

/// Resolved widget — knows its workspace and pre-built wasm dir (if any).
#[derive(Clone)]
pub struct WidgetEntry {
    pub name: String,
    pub workspace: PathBuf,
    /// Pre-built wasm dir, if `--wasm-dir` paired with this workspace.
    /// When `None`, the workspace will be built locally.
    pub wasm_dir: Option<PathBuf>,
}

pub fn execute(args: &RunAllArgs) -> Result<()> {
    let widgets = filter_profile_widgets(
        resolve_widgets(&args.workspaces, &args.wasm_dirs, args.widget.as_deref())?,
        args.stack_profiling,
    );
    ensure_profile_has_widgets(&widgets, args.stack_profiling)?;

    let capture_binary =
        std::env::current_exe().context("failed to resolve capture binary path")?;

    // ── Build (or accept pre-built wasm) ─────────────────────────────
    build_phase(&args.workspaces, &args.wasm_dirs, &widgets)?;

    // ── Capture ─────────────────────────────────────────────────────
    section("Capture");
    let capture_t0 = Instant::now();
    if args.stack_profiling.is_enabled() {
        println!("# WASM stack high-water marks\n");
        println!("| Widget | Maximum observed stack use (bytes) |");
        println!("| --- | ---: |");
    }

    if let Some(n) = args.parallel {
        if widgets.len() > 1 {
            #[expect(clippy::integer_division)]
            let threads = if n == 0 {
                std::thread::available_parallelism().map_or(2, |p| (p.get() / 2).max(1))
            } else {
                n
            };
            capture_parallel(
                &capture_binary,
                &args.output_dir,
                &widgets,
                threads,
                args.stack_profiling,
            )?;
        } else {
            capture_sequential(
                &capture_binary,
                &args.output_dir,
                &widgets,
                args.stack_profiling,
            )?;
        }
    } else {
        capture_sequential(
            &capture_binary,
            &args.output_dir,
            &widgets,
            args.stack_profiling,
        )?;
    }

    let capture_elapsed = capture_t0.elapsed().as_secs_f64();
    let via = super::run::renderer()
        .map(|name| format!(" via {name}"))
        .unwrap_or_default();
    eprintln!(
        "  {}",
        format!("captured in {}{via}", format_time(capture_elapsed)).dimmed()
    );

    Ok(())
}

// ── Discovery + validation ──────────────────────────────────────────

/// Validate the workspaces/wasm-dirs pairing, walk each workspace,
/// and return the flat widget list (filtered by `widget_filter` if set).
pub fn resolve_widgets(
    workspaces: &[PathBuf],
    wasm_dirs: &[PathBuf],
    widget_filter: Option<&str>,
) -> Result<Vec<WidgetEntry>> {
    if workspaces.is_empty() {
        bail!("--workspace: at least one workspace required");
    }
    if !wasm_dirs.is_empty() && wasm_dirs.len() != workspaces.len() {
        bail!(
            "--wasm-dir count ({}) must match --workspace count ({}) when given",
            wasm_dirs.len(),
            workspaces.len(),
        );
    }

    for ws in workspaces {
        if !ws.is_dir() {
            bail!("--workspace: '{}' is not a directory", ws.display());
        }
    }

    let mut all: Vec<WidgetEntry> = Vec::new();
    for (i, ws) in workspaces.iter().enumerate() {
        let names = discover_widgets(ws)?;
        let wasm_dir = wasm_dirs.get(i).cloned();
        for name in names {
            all.push(WidgetEntry {
                name,
                workspace: ws.clone(),
                wasm_dir: wasm_dir.clone(),
            });
        }
    }

    // Globally-unique widget names across workspaces.
    let mut by_name: std::collections::HashMap<&str, Vec<&Path>> = std::collections::HashMap::new();
    for w in &all {
        by_name
            .entry(w.name.as_str())
            .or_default()
            .push(w.workspace.as_path());
    }
    let mut dupes: Vec<(String, Vec<PathBuf>)> = by_name
        .into_iter()
        .filter(|(_, ws)| ws.len() > 1)
        .map(|(n, ws)| {
            (
                n.to_owned(),
                ws.into_iter().map(Path::to_path_buf).collect(),
            )
        })
        .collect();
    dupes.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some((name, ws_paths)) = dupes.first() {
        let where_ = ws_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("widget '{name}' is defined in multiple workspaces: {where_}");
    }

    if let Some(filter) = widget_filter {
        let matches: Vec<WidgetEntry> = all.into_iter().filter(|w| w.name == filter).collect();
        if matches.is_empty() {
            bail!("widget '{filter}' not found in any --workspace");
        }
        return Ok(matches);
    }
    Ok(all)
}

// ── Build phase ─────────────────────────────────────────────────────

/// Render the unified Build section: one `cargo build --workspace`
/// per source-built workspace, plus pre-built validation for
/// any workspace with a paired `--wasm-dir`.
fn build_phase(
    workspaces: &[PathBuf],
    wasm_dirs: &[PathBuf],
    widgets: &[WidgetEntry],
) -> Result<()> {
    section("Build");
    let build_t0 = Instant::now();

    for (i, ws) in workspaces.iter().enumerate() {
        let widget_names: Vec<String> = widgets
            .iter()
            .filter(|w| w.workspace == *ws)
            .map(|w| w.name.clone())
            .collect();
        if widget_names.is_empty() {
            continue;
        }

        let ws_label = workspace_label(ws);
        if let Some(dir) = wasm_dirs.get(i) {
            let asset_root = dir.join("assets");
            if !asset_root.is_dir() {
                bail!(
                    "--wasm-dir: expected package asset root at {}",
                    asset_root.display()
                );
            }
            for name in &widget_names {
                let wasm = wasm_in_dir(dir, name);
                if !wasm.exists() {
                    eprintln!("  {} {}/{}", "✗".red(), ws_label.red().dimmed(), name.red());
                    bail!("--wasm-dir: expected .wasm not found at {}", wasm.display());
                }
                eprintln!(
                    "  {} {}/{}",
                    "✓".green(),
                    ws_label.green().dimmed(),
                    name.green()
                );
            }
        } else {
            build_wasm_workspace(ws, &widget_names)?;
            for name in &widget_names {
                eprintln!(
                    "  {} {}/{}",
                    "✓".green(),
                    ws_label.green().dimmed(),
                    name.green()
                );
            }
        }
    }

    let build_elapsed = build_t0.elapsed().as_secs_f64();
    eprintln!(
        "  {}",
        format!("compiled in {}", format_time(build_elapsed)).dimmed()
    );
    eprintln!();
    Ok(())
}

// ── Widget discovery ────────────────────────────────────────────────

/// Discover all widget crate names under the given workspace.
pub fn discover_widgets(workspace: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(workspace)
        .with_context(|| format!("failed to read {} directory", workspace.display()))?
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

/// Absolute cargo target directory for a workspace, resolved through the
/// shared `scripts/cargo_target_dir.py` tool so the testbed/hot-reload
/// recipes and the capture binary agree on where wasm artifacts land.
fn cargo_target_dir(workspace: &Path) -> Result<PathBuf> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: bmc-wasm-runtime crate must live under the repo root")
        .join("scripts/cargo_target_dir.py");
    let output = Command::new(&script)
        .arg(workspace)
        .output()
        .with_context(|| format!("failed to run {}", script.display()))?;
    if !output.status.success() {
        bail!(
            "{} failed for {}:\n{}",
            script.display(),
            workspace.display(),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    let dir =
        String::from_utf8(output.stdout).context("cargo_target_dir.py emitted non-UTF-8 output")?;
    Ok(PathBuf::from(dir.trim()))
}

/// Resolve the `.wasm` path for a widget within a given directory.
fn wasm_in_dir(dir: &Path, widget: &str) -> PathBuf {
    let name = widget.replace('-', "_");
    dir.join(format!("{name}.wasm"))
}

/// Resolve the `wasmDir` for a widget — either the paired `--wasm-dir`
/// or the workspace's local cargo target dir.
fn wasm_dir_for(entry: &WidgetEntry) -> Result<PathBuf> {
    match &entry.wasm_dir {
        Some(dir) => Ok(dir.clone()),
        None => Ok(cargo_target_dir(&entry.workspace)?
            .join(WASM_TARGET)
            .join("release")),
    }
}

fn asset_root_for(entry: &WidgetEntry) -> Option<PathBuf> {
    entry.wasm_dir.as_ref().map(|dir| dir.join("assets"))
}

/// Widget capture directory (where `config.toml` and fixtures live).
pub fn capture_dir(workspace: &Path, widget: &str) -> PathBuf {
    workspace.join(widget).join("capture")
}

/// Last path component of a workspace, used as the `<workspace>/<widget>`
/// display prefix in status lines and the HTML report.
pub fn workspace_label(workspace: &Path) -> &str {
    workspace
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
}

// ── WASM building ───────────────────────────────────────────────────

/// Build one or all widgets from a single cargo workspace.
///
/// Uses a single `cargo build` from the workspace root — cargo handles
/// parallelism internally, so we don't need to spawn threads.
fn build_wasm_workspace(workspace: &Path, widgets: &[String]) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--release", "--target", WASM_TARGET])
        .current_dir(workspace);

    if widgets.len() == 1 {
        // Select the package by its manifest path, not `-p <dir>`:
        // a widget's crate name can differ from its directory
        // and can collide with a published cargo package.
        let manifest = workspace.join(&widgets[0]).join("Cargo.toml");
        cmd.arg("--manifest-path").arg(&manifest);
    } else {
        cmd.arg("--workspace");
    }

    let output = cmd
        .output()
        .with_context(|| format!("failed to spawn cargo build for {}", workspace.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cargo build failed for {}:\n{stderr}", workspace.display());
    }
    Ok(())
}

// ── Per-example capture ─────────────────────────────────────────────

/// The rasteriser a capture named on its stderr, if the line is there.
fn renderer_from_stderr(stderr: &[u8]) -> Option<&str> {
    std::str::from_utf8(stderr).ok()?.lines().find_map(|line| {
        line.strip_prefix(super::run::RENDERER_PREFIX)
            .map(str::trim)
            .filter(|name| !name.is_empty())
    })
}

/// Capture one widget across every configured target.
/// Returns elapsed seconds and any stack high-water figure.
fn capture_widget(
    binary: &Path,
    output_dir: &Path,
    entry: &WidgetEntry,
    show_progress: bool,
    stack_profiling: StackProfiling,
) -> Result<(f64, Option<u32>)> {
    let example = entry.name.as_str();
    let wasm = wasm_in_dir(&wasm_dir_for(entry)?, example);
    let asset_root = asset_root_for(entry);
    let cap_dir = capture_dir(&entry.workspace, example);
    let output_root = output_dir.join(example).join("current");

    // Wipe previous captures so stale frames don't linger.
    if output_root.exists() {
        let _ = std::fs::remove_dir_all(&output_root);
    }

    // A widget with no config.toml loads an empty config and is skipped below;
    // a config that exists but does not parse is an error, not a silent skip,
    // which would drop the widget out of regression coverage.
    let config = capture_config::load_from_capture_dir(&cap_dir)
        .with_context(|| format!("{example}: capture config"))?;
    let matrix = config.capture_matrix();
    if matrix.is_empty() {
        if stack_profiling.is_enabled() {
            bail!("stack profile requires at least one configured dataset");
        }
        return Ok((0.0, None));
    }

    let t0 = Instant::now();
    let mut stack_high_water = None;

    for (dataset, target) in matrix {
        let output = CaptureConfig::frame_dir(&output_root, dataset, target);
        let label = format!("{target} {dataset}");

        if show_progress {
            progress(&format!("  {example} {label}..."));
        }

        let mut cmd = Command::new(binary);
        cmd.args(["run"])
            .arg(&wasm)
            .arg(format!("--target={target}"))
            .arg(format!("--dataset={dataset}"))
            .arg(format!("--output={}", output.display()))
            .arg(format!("--capture-dir={}", cap_dir.display()));
        if let Some(asset_root) = &asset_root {
            cmd.arg(format!("--asset-root={}", asset_root.display()));
        }
        if stack_profiling.is_enabled() {
            let StackProfiling::Enabled { expected_origin } = stack_profiling else {
                unreachable!("BUG: enabled stack profiling must carry its expected origin");
            };
            cmd.arg(format!("--stack-profile={expected_origin}"));
        }
        // Give a failing run's replay.log the widget's own diagnostics —
        // per-poll telemetry and the click/capture trace, not the frame trail.
        // Respect an explicit RUST_LOG; the terminal stays clean
        // because `distill_capture_error` drops these timeline lines.
        if std::env::var_os("RUST_LOG").is_none() {
            cmd.env("RUST_LOG", "bmc_wasm_runtime=info,capture=debug");
        }

        let result = cmd
            .output()
            .with_context(|| format!("failed to spawn capture for {example} {label}"))?;

        if let Some(name) = renderer_from_stderr(&result.stderr) {
            super::run::note_renderer(name);
        }

        if !result.status.success() {
            // Strip terminal colouring once: the child paints for a TTY,
            // but the captured stderr feeds a file and a line filter here,
            // both of which want plain text.
            let stderr =
                console::strip_ansi_codes(&String::from_utf8_lossy(&result.stderr)).into_owned();
            // Leave a trail a human or a follow-up agent can inspect:
            // the frames the run captured plus the child's full stderr,
            // not only the one-line distilled error. RUST_LOG=debug turns
            // that log into a frame-by-frame replay trace.
            let log_path = output.join("replay.log");
            let _ = std::fs::create_dir_all(&output);
            let _ = std::fs::write(&log_path, &stderr);
            bail!(
                "capture failed for {example} {label}\n{}\n\
                 frames captured: {}\n\
                 replay log:      {}",
                distill_capture_error(&stderr),
                output.display(),
                log_path.display(),
            );
        }
        if stack_profiling.is_enabled() {
            let stdout = String::from_utf8(result.stdout)
                .context("capture stack profile emitted non-UTF-8 output")?;
            let value = stdout
                .lines()
                .filter_map(|line| line.strip_prefix("BMC_STACK_HIGH_WATER="))
                .map(str::parse::<u32>)
                .collect::<Result<Vec<_>, _>>()
                .context("capture stack profile emitted an invalid measurement")?
                .into_iter()
                .max()
                .context("capture stack profile emitted no measurement")?;
            stack_high_water = Some(stack_high_water.map_or(value, |old: u32| old.max(value)));
        }
    }

    if stack_profiling.is_enabled() && stack_high_water.is_none() {
        bail!("stack profile produced no measurement for {example}");
    }

    Ok((t0.elapsed().as_secs_f64(), stack_high_water))
}

fn ensure_profile_has_widgets(
    widgets: &[WidgetEntry],
    stack_profiling: StackProfiling,
) -> Result<()> {
    if widgets.is_empty() && stack_profiling.is_enabled() {
        bail!("stack profile discovered no widgets");
    }
    Ok(())
}

fn filter_profile_widgets(
    widgets: Vec<WidgetEntry>,
    stack_profiling: StackProfiling,
) -> Vec<WidgetEntry> {
    if !stack_profiling.is_enabled() {
        return widgets;
    }
    widgets
        .into_iter()
        .filter(|widget| {
            capture_dir(&widget.workspace, &widget.name)
                .join("config.toml")
                .is_file()
        })
        .collect()
}

/// Strip a failed capture's stderr to the error — drop the GL stack's
/// `pci id …` chatter, per-frame progress, replay headers, timestamped
/// tracing timeline lines (kept in full in `replay.log`), and blanks.
fn distill_capture_error(stderr: &str) -> String {
    let kept = stderr
        .lines()
        .map(str::trim_end)
        .filter(|line| {
            let l = line.trim_start();
            !l.is_empty()
                && !l.starts_with("pci id for fd")
                && !l.starts_with("Captured frame ")
                && !l.starts_with("Unified replay:")
                && !l.starts_with("Capturing ")
                && !l.starts_with(super::run::RENDERER_PREFIX)
                && !is_tracing_line(l)
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Never blank out a failure: if only chatter remains, fall back to raw.
    if kept.is_empty() {
        stderr.trim().to_owned()
    } else {
        kept
    }
}

/// A `tracing` fmt event line, recognised by its leading RFC 3339 timestamp
/// (`2026-…T…Z  LEVEL …`). Dropped from the terminal error, kept in full in
/// `replay.log`.
fn is_tracing_line(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() > 4 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-'
}

fn capture_sequential(
    binary: &Path,
    output_dir: &Path,
    widgets: &[WidgetEntry],
    stack_profiling: StackProfiling,
) -> Result<()> {
    let is_tty = std::io::stderr().is_terminal();
    let mut failures: Vec<(&str, &str, String)> = Vec::new();
    for entry in widgets {
        let ws = workspace_label(&entry.workspace);
        match capture_widget(binary, output_dir, entry, is_tty, stack_profiling) {
            Ok((elapsed, high_water)) => {
                clear_progress();
                widget_status_line(ws, &entry.name, elapsed);
                if let Some(bytes) = high_water {
                    println!("| {} | {bytes} |", entry.name);
                }
            }
            Err(e) => {
                clear_progress();
                eprintln!("  {} {}/{}", "✗".red(), ws.red().dimmed(), entry.name.red());
                failures.push((ws, &entry.name, format!("{e:#}")));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        report_capture_failures(&failures)
    }
}

fn capture_parallel(
    binary: &Path,
    output_dir: &Path,
    widgets: &[WidgetEntry],
    threads: usize,
    stack_profiling: StackProfiling,
) -> Result<()> {
    use rayon::prelude::*;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .context("failed to create thread pool")?;

    let results: Vec<_> = pool.install(|| {
        widgets
            .par_iter()
            .map(|entry| {
                let result = capture_widget(binary, output_dir, entry, false, stack_profiling);
                (
                    workspace_label(&entry.workspace).to_owned(),
                    entry.name.clone(),
                    result,
                )
            })
            .collect()
    });

    // One line per widget here; failure detail goes to its own section below.
    let mut failures: Vec<(&str, &str, String)> = Vec::new();
    for (ws, name, result) in &results {
        match result {
            Ok((elapsed, high_water)) => {
                widget_status_line(ws, name, *elapsed);
                if let Some(bytes) = high_water {
                    println!("| {name} | {bytes} |");
                }
            }
            Err(e) => {
                eprintln!("  {} {}/{}", "✗".red(), ws.red().dimmed(), name.red());
                failures.push((ws, name, format!("{e:#}")));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        report_capture_failures(&failures)
    }
}

/// Print collected capture failures in a framed section and bail. Non-empty.
fn report_capture_failures(failures: &[(&str, &str, String)]) -> Result<()> {
    eprintln!();
    section("Failures");
    for (ws, name, err) in failures {
        eprintln!(
            "  {} {}/{}",
            "✗".red().bold(),
            ws.red().bold(),
            name.red().bold()
        );
        // Header already names the widget; drop the redundant "capture failed for" line.
        for line in err
            .lines()
            .filter(|l| !l.trim_start().starts_with("capture failed for"))
        {
            eprintln!("      {line}");
        }
        eprintln!();
    }
    let (ws, name, _) = &failures[0];
    bail!("capture failed for {ws}/{name}");
}

// ── Display helpers ─────────────────────────────────────────────────

fn widget_status_line(ws: &str, name: &str, elapsed: f64) {
    let time_str = format_time(elapsed);
    let label = format!("  {} {}/{}", "✓".green(), ws.green().dimmed(), name.green());
    let visible_len = 2 + 2 + ws.len() + 1 + name.len() + 1;
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

#[cfg(test)]
mod tests {
    use super::{
        StackProfiling, WidgetEntry, distill_capture_error, ensure_profile_has_widgets,
        filter_profile_widgets, renderer_from_stderr,
    };

    const STACK_PROFILING: StackProfiling = StackProfiling::Enabled {
        expected_origin: 65_536,
    };

    #[test]
    fn distill_keeps_the_error_and_drops_capture_chatter() {
        let raw = "Unified replay: foo (3 events) for iss at 1280x480\n\
                   pci id for fd 9: 10de:2684, driver (null)\n\
                   Capturing iss at 1280x480 (SDK 0.1.0)\n\
                   Captured frame 0 → /x/frame_0000.png\n\
                   error: hermetic capture breach in iss (1280x480):\n\
                   error:   fetch: GET https://x/y\n";
        assert_eq!(
            distill_capture_error(raw),
            "error: hermetic capture breach in iss (1280x480):\n\
             error:   fetch: GET https://x/y"
        );
    }

    #[test]
    fn distill_falls_back_to_raw_when_only_chatter() {
        // A failure whose output is entirely chatter (or an unexpected shape)
        // must still surface something — never an empty detail that hides it.
        let raw = "pci id for fd 9: 10de:2684, driver (null)\n\
                   Captured frame 0 → /x/frame_0000.png\n";
        assert_eq!(distill_capture_error(raw), raw.trim());
    }

    /// The marker is the only route out of a capture's own process, so the
    /// prefix and the name it carries are a contract between the two.
    #[test]
    fn the_renderer_marker_is_read_off_a_capture_line() {
        let raw = b"Capturing iss at 1280x480 (SDK 0.1.0)\n\
                    renderer: llvmpipe (LLVM 21.1.8, 256 bits)\n" as &[u8];
        assert_eq!(
            renderer_from_stderr(raw),
            Some("llvmpipe (LLVM 21.1.8, 256 bits)")
        );
    }

    #[test]
    fn stderr_without_a_renderer_marker_names_nothing() {
        assert_eq!(renderer_from_stderr(b"Captured frame 0\n"), None);
        assert_eq!(renderer_from_stderr(b"renderer: \n"), None);
    }

    #[test]
    fn the_renderer_marker_is_dropped_from_a_distilled_failure() {
        let raw = "renderer: llvmpipe (LLVM 21.1.8, 256 bits)\n\
                   error: hermetic capture breach in iss (1280x480):\n";
        assert_eq!(
            distill_capture_error(raw),
            "error: hermetic capture breach in iss (1280x480):"
        );
    }

    #[test]
    fn stack_profile_rejects_an_empty_widget_set() {
        assert!(ensure_profile_has_widgets(&[], STACK_PROFILING).is_err());
        ensure_profile_has_widgets(&[], StackProfiling::Disabled)
            .expect("ordinary capture may select no widgets");
    }

    #[test]
    fn stack_profile_skips_widgets_without_a_capture_config() {
        let temp = tempfile::tempdir().expect("temporary workspace must be created");
        let configured_capture = temp.path().join("configured/capture");
        std::fs::create_dir_all(&configured_capture)
            .expect("configured widget directory must be writable");
        std::fs::write(configured_capture.join("config.toml"), "")
            .expect("capture config must be writable");
        let widgets = ["configured", "unconfigured"].map(|name| WidgetEntry {
            name: name.to_owned(),
            workspace: temp.path().to_owned(),
            wasm_dir: None,
        });

        let filtered = filter_profile_widgets(widgets.to_vec(), STACK_PROFILING);

        assert_eq!(
            filtered
                .into_iter()
                .map(|widget| widget.name)
                .collect::<Vec<_>>(),
            ["configured"]
        );
    }
}

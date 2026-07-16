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

//! `diff` subcommand — compare current captures against baselines using odiff.
//!
//! Operates on a single widget. The `verify` orchestrator calls this for each
//! widget, then aggregates results into an HTML report.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use askama::Template;
use base64::Engine;
use owo_colors::OwoColorize;

use super::tools::resolve_tool;

// ── Public interface ────────────────────────────────────────────────

pub struct DiffArgs {
    /// Workspace prefix for status-line / report display.
    /// Empty when the caller has no workspace context (standalone `diff`).
    pub workspace: String,
    /// Path to the `capture/` directory containing `baselines.7z`.
    pub capture_dir: PathBuf,
    /// Path to the widget's output directory (contains `current/`, `diff/`).
    pub output: PathBuf,
    /// Color distance threshold (0.0 = exact match).
    pub threshold: f64,
    /// Suppress per-image progress output (for parallel execution).
    pub quiet_progress: bool,
}

/// Run diff for a single widget. Returns the report, the baseline temp dir
/// (kept alive until the caller drops it), and elapsed seconds.
///
/// The caller prints `widget_status_line` — so a parallel orchestrator
/// can collect and print in input order rather than thread-completion order.
pub fn execute(args: &DiffArgs) -> Result<(WidgetReport, Option<tempfile::TempDir>, f64)> {
    let odiff_bin = resolve_tool("odiff", "odiff")?;
    let t0 = Instant::now();
    let (report, baseline_tmp) = diff_one_widget(args, &odiff_bin, args.quiet_progress)?;
    let elapsed = t0.elapsed().as_secs_f64();
    Ok((report, baseline_tmp, elapsed))
}

// ── Per-widget diff ─────────────────────────────────────────────────

fn diff_one_widget(
    args: &DiffArgs,
    odiff_bin: &Path,
    quiet: bool,
) -> Result<(WidgetReport, Option<tempfile::TempDir>)> {
    let baseline_archive = args.capture_dir.join("baselines.7z");
    let current_dir = args.output.join("current");
    let diff_dir = args.output.join("diff");

    let widget_name = args
        .output
        .file_name()
        .map_or("widget".to_owned(), |n| n.to_string_lossy().into_owned());

    if !baseline_archive.exists() || !current_dir.exists() {
        return Ok((
            WidgetReport {
                workspace: args.workspace.clone(),
                widget: widget_name,
                no_baseline: true,
                ..Default::default()
            },
            None,
        ));
    }

    // Extract baseline to temp dir
    let tmp = tempfile::tempdir().context("failed to create temp dir")?;
    sevenz_rust2::decompress_file(&baseline_archive, tmp.path())
        .with_context(|| format!("failed to extract baseline {}", baseline_archive.display()))?;

    // Wipe previous diff output
    if diff_dir.exists() {
        let _ = std::fs::remove_dir_all(&diff_dir);
    }

    let report = diff_directories(
        tmp.path(),
        &current_dir,
        &diff_dir,
        args.threshold,
        WidgetLabel {
            workspace: &args.workspace,
            widget: &widget_name,
        },
        odiff_bin,
        quiet,
    )?;

    Ok((report, Some(tmp)))
}

// ── Directory comparison ────────────────────────────────────────────

/// Display labels for one widget — `<workspace>/<widget>`
/// shows up in status lines and the HTML report.
#[derive(Copy, Clone)]
struct WidgetLabel<'a> {
    workspace: &'a str,
    widget: &'a str,
}

fn diff_directories(
    baseline_dir: &Path,
    current_dir: &Path,
    diff_dir: &Path,
    threshold: f64,
    label: WidgetLabel<'_>,
    odiff_bin: &Path,
    quiet: bool,
) -> Result<WidgetReport> {
    let mut report = WidgetReport {
        workspace: label.workspace.to_owned(),
        widget: label.widget.to_owned(),
        ..Default::default()
    };

    // Collect baseline PNGs
    let baseline_pngs = collect_pngs(baseline_dir);
    let mut baseline_rels = HashSet::new();

    if baseline_pngs.is_empty() {
        return Ok(report);
    }

    // Spawn odiff server
    let mut server = OdiffServer::spawn(odiff_bin, threshold)?;

    for (i, baseline_file) in baseline_pngs.iter().enumerate() {
        let rel = baseline_file
            .strip_prefix(baseline_dir)
            .unwrap_or(baseline_file);
        let rel_str = rel.to_string_lossy().into_owned();
        baseline_rels.insert(rel.to_path_buf());
        let current_file = current_dir.join(rel);
        let diff_file = diff_dir.join(rel);

        if !quiet {
            progress(&format!("  {}/{}", i + 1, baseline_pngs.len()));
        }

        if !current_file.exists() {
            report.results.push(DiffResult {
                rel: rel_str,
                status: "missing".into(),
                baseline_file: Some(baseline_file.clone()),
                ..Default::default()
            });
            report.missing += 1;
            continue;
        }

        // Ensure diff output dir
        if let Some(parent) = diff_file.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let cmp = server.compare(baseline_file, &current_file, &diff_file)?;

        if cmp.matched {
            report.results.push(DiffResult {
                rel: rel_str,
                status: "pass".into(),
                baseline_file: Some(baseline_file.clone()),
                current_file: Some(current_file),
                ..Default::default()
            });
            report.passed += 1;
            // Clean up empty diff dirs (odiff doesn't write on pass)
            cleanup_empty_parents(&diff_file, diff_dir);
        } else {
            report.results.push(DiffResult {
                rel: rel_str,
                status: "diff".into(),
                diff_percentage: cmp.diff_percentage,
                baseline_file: Some(baseline_file.clone()),
                current_file: Some(current_file),
                diff_file: Some(diff_file),
            });
            report.failed += 1;
        }
    }

    // Detect NEW files in current that have no baseline counterpart
    for current_file in collect_pngs(current_dir) {
        let rel = current_file
            .strip_prefix(current_dir)
            .unwrap_or(&current_file);
        if !baseline_rels.contains(rel) {
            report.results.push(DiffResult {
                rel: rel.to_string_lossy().into_owned(),
                status: "new".into(),
                current_file: Some(current_file),
                ..Default::default()
            });
            report.new_count += 1;
        }
    }

    if !quiet {
        clear_progress();
    }
    Ok(report)
}

fn collect_pngs(dir: &Path) -> Vec<PathBuf> {
    let mut pngs = Vec::new();
    collect_pngs_recursive(dir, &mut pngs);
    pngs.sort();
    pngs
}

fn collect_pngs_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_pngs_recursive(&path, out);
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}

fn cleanup_empty_parents(file: &Path, stop_at: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == stop_at {
            break;
        }
        if std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

// ── odiff server ────────────────────────────────────────────────────

/// Long-lived odiff process using `--server` mode (JSON over stdin/stdout).
struct OdiffServer {
    child: Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
    threshold: f64,
}

impl OdiffServer {
    fn spawn(odiff_bin: &Path, threshold: f64) -> Result<Self> {
        let mut child = Command::new(odiff_bin)
            .arg("--server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn {}", odiff_bin.display()))?;

        let stdin = child.stdin.take().expect("BUG: stdin not piped");
        let stdout = child.stdout.take().expect("BUG: stdout not piped");
        let mut reader = BufReader::new(stdout);

        // Read the ready message
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("failed to read odiff ready message")?;
        if !line.contains("\"ready\"") {
            bail!("unexpected odiff server greeting: {line}");
        }

        Ok(Self {
            child,
            stdin,
            reader,
            next_id: 1,
            threshold,
        })
    }

    fn compare(&mut self, base: &Path, compare: &Path, output: &Path) -> Result<OdiffResult> {
        let id = self.next_id;
        self.next_id += 1;

        let mut request = serde_json::json!({
            "requestId": id,
            "base": base.display().to_string(),
            "compare": compare.display().to_string(),
            "output": output.display().to_string(),
            "options": {
                "threshold": self.threshold,
                "diffOverlay": 0.1,
            },
        })
        .to_string();
        request.push('\n');

        self.stdin
            .write_all(request.as_bytes())
            .context("failed to write to odiff server")?;
        self.stdin
            .flush()
            .context("failed to flush odiff server stdin")?;

        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .context("failed to read odiff server response")?;

        if line.contains("\"error\"") {
            bail!("odiff error: {}", line.trim());
        }

        let matched = line.contains("\"match\":true");
        let diff_percentage = parse_json_f64(&line, "diffPercentage").unwrap_or(0.0);

        Ok(OdiffResult {
            matched,
            diff_percentage,
        })
    }
}

impl Drop for OdiffServer {
    fn drop(&mut self) {
        // Close stdin to signal EOF, then kill + wait to clean up
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── odiff result ────────────────────────────────────────────────────

struct OdiffResult {
    matched: bool,
    diff_percentage: f64,
}

/// Extract a float value from a JSON string by key.
fn parse_json_f64(json: &str, key: &str) -> Option<f64> {
    let parsed: serde_json::Value = serde_json::from_str(json).ok()?;
    parsed.get(key)?.as_f64()
}

// ── Report types (exported for verify orchestrator) ─────────────────

#[derive(Default)]
pub struct WidgetReport {
    /// Last path component of the widget's workspace dir, e.g. `examples`,
    /// `widgets-wasm`. Empty when produced by a standalone `diff` invocation
    /// that has no workspace context.
    pub workspace: String,
    pub widget: String,
    pub results: Vec<DiffResult>,
    pub passed: u32,
    pub failed: u32,
    pub missing: u32,
    pub new_count: u32,
    pub no_baseline: bool,
}

impl WidgetReport {
    pub fn has_failures(&self) -> bool {
        self.no_baseline || self.failed > 0 || self.missing > 0 || self.new_count > 0
    }
}

#[derive(Default)]
pub struct DiffResult {
    pub rel: String,
    pub status: String,
    pub diff_percentage: f64,
    pub baseline_file: Option<PathBuf>,
    pub current_file: Option<PathBuf>,
    pub diff_file: Option<PathBuf>,
}

// ── HTML report (exported for verify orchestrator) ──────────────────

#[derive(Template)]
#[template(path = "report.askama.html")]
struct ReportTemplate<'a> {
    reports: &'a [WidgetReportView],
    total_pass: u32,
    total_fail: u32,
    total_missing: u32,
    total_new: u32,
    no_baseline_count: u32,
    has_failures: bool,
}

/// View model for the template — owns the data URI strings.
struct WidgetReportView {
    workspace: String,
    widget: String,
    results: Vec<DiffResultView>,
    passed: u32,
    failed: u32,
    missing: u32,
    new_count: u32,
    no_baseline: bool,
    /// Total comparable frames (passed + failed + missing + new). Used for the
    /// "n/total" headline on failing widgets.
    total: u32,
    /// True when nothing went wrong — used to decide whether the widget's
    /// `<details>` opens by default.
    is_clean: bool,
    /// Status class for the minimap pill: `"pass"`, `"fail"`, or `"no-baseline"`.
    minimap_status: &'static str,
    /// Compact count label shown next to the pill.
    minimap_count: String,
}

struct DiffResultView {
    rel: String,
    status: String,
    diff_percentage: f64,
    baseline_uri: Option<String>,
    current_uri: Option<String>,
    diff_uri: Option<String>,
}

fn img_to_data_uri(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
    Some(format!("data:image/png;base64,{b64}"))
}

pub fn generate_html_report(reports: &[WidgetReport], output: &Path) -> Result<()> {
    let total_pass: u32 = reports.iter().map(|r| r.passed).sum();
    let total_fail: u32 = reports.iter().map(|r| r.failed).sum();
    let total_missing: u32 = reports.iter().map(|r| r.missing).sum();
    let total_new: u32 = reports.iter().map(|r| r.new_count).sum();
    let no_baseline_count = reports.iter().filter(|r| r.no_baseline).count() as u32;
    let has_failures =
        total_fail > 0 || total_missing > 0 || total_new > 0 || no_baseline_count > 0;

    // Build view models with embedded data URIs for non-pass results
    let views: Vec<WidgetReportView> = reports
        .iter()
        .map(|r| {
            let results = r
                .results
                .iter()
                .map(|d| {
                    let (baseline_uri, current_uri, diff_uri) = if d.status == "pass" {
                        (None, None, None)
                    } else {
                        (
                            d.baseline_file.as_deref().and_then(img_to_data_uri),
                            d.current_file.as_deref().and_then(img_to_data_uri),
                            d.diff_file.as_deref().and_then(img_to_data_uri),
                        )
                    };
                    DiffResultView {
                        rel: d.rel.clone(),
                        status: d.status.clone(),
                        diff_percentage: d.diff_percentage,
                        baseline_uri,
                        current_uri,
                        diff_uri,
                    }
                })
                .collect();

            let total = r.passed + r.failed + r.missing + r.new_count;
            let is_clean = !r.no_baseline && r.failed == 0 && r.missing == 0 && r.new_count == 0;
            let (minimap_status, minimap_count) = if r.no_baseline {
                ("no-baseline", "—".to_owned())
            } else if is_clean {
                ("pass", r.passed.to_string())
            } else {
                ("fail", format!("{}/{total}", r.failed))
            };
            WidgetReportView {
                workspace: r.workspace.clone(),
                widget: r.widget.clone(),
                results,
                passed: r.passed,
                failed: r.failed,
                missing: r.missing,
                new_count: r.new_count,
                no_baseline: r.no_baseline,
                total,
                is_clean,
                minimap_status,
                minimap_count,
            }
        })
        .collect();

    let template = ReportTemplate {
        reports: &views,
        total_pass,
        total_fail,
        total_missing,
        total_new,
        no_baseline_count,
        has_failures,
    };

    let html = template.render().context("failed to render HTML report")?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, html)
        .with_context(|| format!("failed to write report to {}", output.display()))?;

    let abs_path = std::fs::canonicalize(output).unwrap_or_else(|_| output.to_owned());
    eprintln!("\nHTML report: {}", abs_path.display());
    Ok(())
}

// ── A/B comparison media (exported for verify orchestrator) ─────────

/// Wide sizes are stacked vertically (A on top, B below);
/// square-ish sizes are placed side by side.
const VERTICAL_SIZES: &[&str] = &["full", "medium"];

fn comparison_filter(size: &str) -> String {
    let stack = if VERTICAL_SIZES.contains(&size) {
        "vstack"
    } else {
        "hstack"
    };
    format!("[0][1]{stack}")
}

struct ComparisonJob<'a> {
    widget: &'a str,
    size: String,
    results: Vec<&'a DiffResult>,
    out_path: PathBuf,
}

/// Generate A/B comparison media (PNG for single frame, video for animations).
pub fn generate_comparisons(reports: &[WidgetReport], output_dir: &Path) -> Result<()> {
    use rayon::prelude::*;

    let failed_reports: Vec<_> = reports.iter().filter(|r| r.failed > 0).collect();
    if failed_reports.is_empty() {
        return Ok(());
    }

    let ffmpeg_bin = resolve_tool("ffmpeg", "ffmpeg")?;

    eprintln!();
    section("Video");
    let total_t0 = Instant::now();

    let mut jobs = Vec::new();
    for report in &failed_reports {
        let mut sizes: std::collections::BTreeMap<String, Vec<&DiffResult>> =
            std::collections::BTreeMap::new();
        for result in &report.results {
            if result.status != "diff" {
                continue;
            }
            if let Some(size) = result.rel.split('/').next() {
                sizes.entry(size.to_owned()).or_default().push(result);
            }
        }
        let out_dir = output_dir.join(&report.widget);
        for (size, results) in sizes {
            let ext = if results.len() == 1 { "png" } else { "mp4" };
            let out_path = out_dir.join(format!("{size}.{ext}"));
            jobs.push(ComparisonJob {
                widget: &report.widget,
                size,
                results,
                out_path,
            });
        }
    }

    // Ensure output dirs exist before parallel work.
    for job in &jobs {
        if let Some(parent) = job.out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Run all ffmpeg jobs in parallel.
    let errors: Vec<_> = jobs
        .par_iter()
        .filter_map(|job| {
            let result = if job.results.len() == 1 {
                render_comparison_image(&ffmpeg_bin, job.results[0], &job.size, &job.out_path)
            } else {
                render_comparison_video(&ffmpeg_bin, &job.results, &job.size, &job.out_path)
            };
            result
                .err()
                .map(|e| format!("{} {}: {e:#}", job.widget, job.size))
        })
        .collect();

    // Print per-widget summaries.
    for report in &failed_reports {
        let count = jobs.iter().filter(|j| j.widget == report.widget).count();
        let label = format!("  {} {}", "\u{2713}".green(), report.widget.green());
        let right = format!("{count} {}", if count == 1 { "file" } else { "files" });
        let visible_label = 2 + 2 + report.widget.len() + 1;
        let dots = COL_WIDTH.saturating_sub(visible_label + right.len()).max(1);
        eprintln!("{label} {} {right}", "·".repeat(dots).dimmed());
    }

    let total_elapsed = total_t0.elapsed().as_secs_f64();
    eprintln!("  {}", format!("rendered in {total_elapsed:.1}s").dimmed());

    if !errors.is_empty() {
        bail!("ffmpeg failures:\n{}", errors.join("\n"));
    }
    Ok(())
}

/// Render a single A/B composite PNG (baseline + diff).
fn render_comparison_image(
    ffmpeg: &Path,
    result: &DiffResult,
    size: &str,
    output: &Path,
) -> Result<()> {
    let (Some(baseline), Some(diff)) = (&result.baseline_file, &result.diff_file) else {
        return Ok(());
    };
    let filter = comparison_filter(size);
    let status = Command::new(ffmpeg)
        .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
        .arg(baseline)
        .arg("-i")
        .arg(diff)
        .args(["-filter_complex", &filter])
        .arg(output)
        .status()
        .context("failed to spawn ffmpeg")?;
    if !status.success() {
        bail!(
            "ffmpeg comparison image failed (exit {})",
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

/// Render an A/B comparison video from diff results in a single ffmpeg call.
///
/// Instead of spawning N+1 processes (composite each frame, then concat+encode),
/// we build one filtergraph that composites every pair, holds each still for 0.5 s,
/// concatenates the streams, and encodes to H.264 — one fork total.
/// 30 fps × 0.5 s = 15 frames per still.
const VIDEO_FPS: u32 = 30;
const VIDEO_HOLD_FRAMES: u32 = 15;

fn render_comparison_video(
    ffmpeg: &Path,
    results: &[&DiffResult],
    size: &str,
    output: &Path,
) -> Result<()> {
    use std::fmt::Write;

    let stack = if VERTICAL_SIZES.contains(&size) {
        "vstack"
    } else {
        "hstack"
    };

    // Collect valid (baseline, diff) pairs in sorted order.
    let mut pairs: Vec<_> = results
        .iter()
        .filter_map(|r| match (&r.baseline_file, &r.diff_file) {
            (Some(b), Some(d)) => Some((b.as_path(), d.as_path())),
            _ => None,
        })
        .collect();
    pairs.sort_by_key(|(_, d)| d.to_path_buf());

    if pairs.is_empty() {
        return Ok(());
    }

    // Build -i args: baseline0 diff0 baseline1 diff1 …
    // `-nostdin` so ffmpeg doesn't enter its interactive console when one of
    // the parallel jobs happens to inherit a TTY stdin (which surfaces as
    // `Enter command: <target>|all <time>|-1 <command>[ <argument>]` blocking
    // the verify run).
    let mut args: Vec<std::ffi::OsString> = vec![
        "-nostdin".into(),
        "-y".into(),
        "-loglevel".into(),
        "error".into(),
    ];
    for (baseline, diff) in &pairs {
        args.extend(["-i".into(), baseline.as_os_str().to_owned()]);
        args.extend(["-i".into(), diff.as_os_str().to_owned()]);
    }

    // Build filtergraph:
    //   [0][1]hstack[f0];[f0]loop=15:1:0,fps=30[s0];
    //   [2][3]hstack[f1];[f1]loop=15:1:0,fps=30[s1];
    //   …
    //   [s0][s1]…[sN]concat=n=N:v=1:a=0,pad=…
    let n = pairs.len();
    let mut fg = String::new();
    for i in 0..n {
        let inp_base = i * 2;
        let inp_diff = inp_base + 1;
        write!(
            fg,
            "[{inp_base}][{inp_diff}]{stack}[f{i}];\
             [f{i}]loop={VIDEO_HOLD_FRAMES}:1:0,fps={VIDEO_FPS}[s{i}];",
        )
        .expect("BUG: fmt write to String");
    }
    // concat all streams
    for i in 0..n {
        write!(fg, "[s{i}]").expect("BUG: fmt write to String");
    }
    write!(fg, "concat=n={n}:v=1:a=0,pad=ceil(iw/2)*2:ceil(ih/2)*2")
        .expect("BUG: fmt write to String");

    args.extend(["-filter_complex".into(), fg.into()]);
    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-profile:v".into(),
        "high".into(),
        "-level".into(),
        "4.1".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-crf".into(),
        "18".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    args.push(output.as_os_str().to_owned());

    let status = Command::new(ffmpeg)
        .args(&args)
        .status()
        .context("failed to spawn ffmpeg for comparison video")?;
    if !status.success() {
        bail!(
            "ffmpeg comparison video failed (exit {})",
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}

// ── Terminal output ─────────────────────────────────────────────────

const COL_WIDTH: usize = 50;

/// Visible width of a string (character count, not byte count).
fn visible_len(s: &str) -> usize {
    s.chars().count()
}

pub fn section(title: &str) {
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

pub fn widget_status_line(report: &WidgetReport, elapsed: f64) -> String {
    let total = report.passed + report.failed + report.missing + report.new_count;
    let ok =
        !report.no_baseline && report.failed == 0 && report.missing == 0 && report.new_count == 0;

    let (mark, right_text) = if report.no_baseline {
        ("\u{2717}", "no baseline".to_owned())
    } else if !ok {
        let mut parts = Vec::new();
        if report.failed > 0 {
            let max_pct = report
                .results
                .iter()
                .filter(|r| r.status == "diff")
                .map(|r| r.diff_percentage)
                .fold(0.0_f64, f64::max);
            parts.push(format!("{}Δ ↑{max_pct:.2}%", report.failed));
        }
        if report.missing > 0 {
            parts.push(format!("{} missing", report.missing));
        }
        if report.new_count > 0 {
            parts.push(format!("{} new", report.new_count));
        }
        ("\u{2717}", parts.join(", "))
    } else {
        ("\u{2713}", total.to_string())
    };

    let time_str = format!("{elapsed:.1}s");
    let ws = report.workspace.as_str();
    let name = report.widget.as_str();
    let has_ws = !ws.is_empty();
    let visible_label = if has_ws {
        2 + 2 + ws.len() + 1 + name.len() + 1
    } else {
        2 + 2 + name.len() + 1
    };
    let visible_right = 1 + visible_len(&right_text) + 2 + time_str.len();
    let dots = COL_WIDTH
        .saturating_sub(visible_label + visible_right)
        .max(1);

    if ok {
        let label = if has_ws {
            format!("{} {}/{}", mark.green(), ws.green().dimmed(), name.green())
        } else {
            format!("{} {}", mark.green(), name.green())
        };
        format!(
            "  {label} {} {} {}",
            "·".repeat(dots).dimmed(),
            right_text.green(),
            time_str.dimmed()
        )
    } else {
        let label = if has_ws {
            format!("{} {}/{}", mark.red(), ws.red().dimmed(), name.red())
        } else {
            format!("{} {}", mark.red(), name.red())
        };
        format!(
            "  {label} {} {} {}",
            "·".repeat(dots).dimmed(),
            right_text.red(),
            time_str.dimmed()
        )
    }
}

pub fn print_failure_details(reports: &[WidgetReport]) {
    let mut lines = Vec::new();
    for report in reports {
        let label = if report.workspace.is_empty() {
            report.widget.clone()
        } else {
            format!("{}/{}", report.workspace, report.widget)
        };
        if report.no_baseline {
            lines.push(format!(
                "  {} {label} — run `capture set-baseline`",
                "NO BASELINE:".red(),
            ));
            continue;
        }
        let total = report.failed + report.missing + report.new_count;
        if total == 0 {
            continue;
        }
        let mut parts = Vec::new();
        if report.failed > 0 {
            let max_pct = report
                .results
                .iter()
                .filter(|r| r.status == "diff")
                .map(|r| r.diff_percentage)
                .fold(0.0_f64, f64::max);
            parts.push(format!("Δ {} {max_pct:.2}%", report.failed));
        }
        if report.missing > 0 {
            parts.push(format!("{} missing", report.missing));
        }
        if report.new_count > 0 {
            parts.push(format!("{} new", report.new_count));
        }
        lines.push(format!(
            "  {} {}",
            format!("{label}:").red(),
            parts.join(", ").dimmed()
        ));
    }
    if !lines.is_empty() {
        eprintln!();
        for line in &lines {
            eprintln!("{line}");
        }
    }
}

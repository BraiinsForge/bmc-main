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
    /// Isolated pixels a frame may differ by before it counts as a failure.
    pub max_diff_pixels: usize,
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
    // A video the previous run left reads exactly like a fresh one.
    let _ = std::fs::remove_dir_all(args.output.join(super::media::Media::Comparison.dir()));

    let report = diff_directories(
        tmp.path(),
        &current_dir,
        &diff_dir,
        Limits {
            threshold: args.threshold,
            max_diff_pixels: args.max_diff_pixels,
        },
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

/// What a frame may differ by: colour distance per pixel, and the pixels
/// [`verdict`] absorbs on top of it.
#[derive(Copy, Clone)]
struct Limits {
    threshold: f64,
    max_diff_pixels: usize,
}

fn diff_directories(
    baseline_dir: &Path,
    current_dir: &Path,
    diff_dir: &Path,
    limits: Limits,
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
    let mut server = OdiffServer::spawn(odiff_bin, limits.threshold)?;

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

        match verdict(&cmp, limits.max_diff_pixels) {
            Verdict::Pass => {
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
            }
            // Keeps odiff's diff image, so tolerated drift can be looked at.
            Verdict::Tolerated => {
                report.results.push(DiffResult {
                    rel: rel_str,
                    status: "tolerated".into(),
                    diff: cmp.diff,
                    baseline_file: Some(baseline_file.clone()),
                    current_file: Some(current_file),
                    diff_file: Some(diff_file),
                });
                report.tolerated += 1;
            }
            Verdict::Fail => {
                report.results.push(DiffResult {
                    rel: rel_str,
                    status: "diff".into(),
                    diff: cmp.diff,
                    baseline_file: Some(baseline_file.clone()),
                    current_file: Some(current_file),
                    diff_file: Some(diff_file),
                });
                report.failed += 1;
            }
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
                "failOnLayoutDiff": false,
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

        parse_odiff_response(&line)
    }
}

fn parse_odiff_response(line: &str) -> Result<OdiffResult> {
    let parsed: serde_json::Value =
        serde_json::from_str(line).context("failed to parse odiff response")?;
    if let Some(error) = parsed.get("error") {
        bail!("odiff error: {error}");
    }

    let matched = parsed
        .get("match")
        .and_then(serde_json::Value::as_bool)
        .context("odiff response has no \"match\" field")?;
    let diff = if matched {
        None
    } else {
        if parsed.get("reason").and_then(serde_json::Value::as_str) == Some("layout-diff") {
            bail!("odiff returned unexpected layout-diff with failOnLayoutDiff disabled");
        }
        let count = parsed
            .get("diffCount")
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .context("odiff reported a mismatch without a usable diffCount")?;
        let percentage = parsed
            .get("diffPercentage")
            .and_then(serde_json::Value::as_f64)
            .context("odiff reported a mismatch without a diffPercentage")?;
        Some(PixelDiff { count, percentage })
    };

    Ok(OdiffResult { matched, diff })
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
    /// `None` exactly when the frames matched.
    diff: Option<PixelDiff>,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Pass,
    /// Differs, but by no more pixels than the budget.
    Tolerated,
    Fail,
}

/// Judge one frame against a budget of differing pixels.
///
/// Rasterisers disagree on which side of a pixel centre a steep antialiased edge falls.
/// That flip is line colour against background: a full contrast step,
/// which no colour-distance threshold can absorb.
/// ANGLE and llvmpipe differ on one pixel per affected braiins-pool frame,
/// while a moved line or a changed glyph runs to hundreds.
///
/// A count, with no test for where the pixels sit: odiff reports how many
/// differ, not which, so a cluster of them spends the budget as freely as
/// a scattering. The gap between the two is far below the hundreds a real
/// visual change costs.
///
/// Only a colour difference reaches here — [`parse_odiff_response`] rejects a
/// layout difference outright — so the budget can never absorb a size change.
fn verdict(cmp: &OdiffResult, budget: usize) -> Verdict {
    if cmp.matched {
        return Verdict::Pass;
    }
    match cmp.diff {
        Some(diff) if diff.count <= budget => Verdict::Tolerated,
        // `parse_odiff_response` refuses a mismatch without measurements.
        _ => Verdict::Fail,
    }
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
    /// Frames within the pixel budget — not failures, but reported.
    pub tolerated: u32,
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

/// How much of a frame changed. Carried only by frames that actually differ.
#[derive(Clone, Copy)]
pub struct PixelDiff {
    pub count: usize,
    pub percentage: f64,
}

#[derive(Default)]
pub struct DiffResult {
    pub rel: String,
    pub status: String,
    pub diff: Option<PixelDiff>,
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
    total_tolerated: u32,
    total_fail: u32,
    total_missing: u32,
    total_new: u32,
    no_baseline_count: u32,
    has_failures: bool,
    /// Absent when no capture ran in this process, as for a standalone `diff`.
    renderer: Option<&'a str>,
}

/// View model for the template — owns the data URI strings.
struct WidgetReportView {
    workspace: String,
    widget: String,
    results: Vec<DiffResultView>,
    passed: u32,
    tolerated: u32,
    failed: u32,
    missing: u32,
    new_count: u32,
    no_baseline: bool,
    /// Total comparable frames — the "n/total" headline on failing widgets.
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
    diff_count: usize,
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
    let total_tolerated: u32 = reports.iter().map(|r| r.tolerated).sum();
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
                        diff_count: d.diff.map_or(0, |diff| diff.count),
                        diff_percentage: d.diff.map_or(0.0, |diff| diff.percentage),
                        baseline_uri,
                        current_uri,
                        diff_uri,
                    }
                })
                .collect();

            let total = r.passed + r.tolerated + r.failed + r.missing + r.new_count;
            let is_clean = !r.no_baseline && r.failed == 0 && r.missing == 0 && r.new_count == 0;
            let (minimap_status, minimap_count) = if r.no_baseline {
                ("no-baseline", "—".to_owned())
            } else if is_clean {
                ("pass", total.to_string())
            } else {
                ("fail", format!("{}/{total}", r.failed))
            };
            WidgetReportView {
                workspace: r.workspace.clone(),
                widget: r.widget.clone(),
                results,
                passed: r.passed,
                tolerated: r.tolerated,
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
        total_tolerated,
        total_fail,
        total_missing,
        total_new,
        no_baseline_count,
        has_failures,
        renderer: super::run::renderer(),
    };

    let html = template.render().context("failed to render HTML report")?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Write through a sibling and rename: the report's existence is taken as
    // proof that a verdict was reached, so one truncated by a full disk would
    // be read as a visual regression rather than the failure it is.
    let staging = output.with_extension("html.part");
    std::fs::write(&staging, html)
        .with_context(|| format!("failed to write report to {}", staging.display()))?;
    std::fs::rename(&staging, output)
        .with_context(|| format!("failed to move report into {}", output.display()))?;

    let abs_path = std::fs::canonicalize(output).unwrap_or_else(|_| output.to_owned());
    eprintln!("\nHTML report: {}", abs_path.display());
    Ok(())
}

// ── A/B comparison media (exported for verify orchestrator) ─────────

#[derive(Clone, Copy)]
enum Stack {
    Vertical,
    Horizontal,
}

impl Stack {
    /// A wide frame stacks vertically, so the pair stays near square.
    fn for_frame((width, height): (u32, u32)) -> Self {
        if width > height {
            Self::Vertical
        } else {
            Self::Horizontal
        }
    }

    fn filter(self) -> &'static str {
        match self {
            Self::Vertical => "vstack",
            Self::Horizontal => "hstack",
        }
    }
}

/// The directory a frame was captured into: one dataset on one target,
/// so every frame in it shares a size.
fn frame_group(rel: &str) -> Option<&Path> {
    Path::new(rel)
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
}

/// The size every frame in a group shares.
///
/// ffmpeg answers unequal inputs with an exit code over a filtergraph,
/// so reading the headers first names the odd frame.
fn common_frame_size(pairs: &[(&Path, &Path)]) -> Result<(u32, u32)> {
    let mut agreed: Option<((u32, u32), &Path)> = None;
    for path in pairs.iter().flat_map(|(baseline, diff)| [*baseline, *diff]) {
        let size = image::image_dimensions(path)
            .with_context(|| format!("failed to read the size of {}", path.display()))?;
        match agreed {
            Some((first_size, first)) if first_size != size => bail!(
                "frames differ in size: {} is {}×{}, {} is {}×{}",
                first.display(),
                first_size.0,
                first_size.1,
                path.display(),
                size.0,
                size.1,
            ),
            Some(_) => {}
            None => agreed = Some((size, path)),
        }
    }
    let (size, _) = agreed.expect("BUG: a group exists only because a pair went into it");
    Ok(size)
}

struct ComparisonJob<'a> {
    widget: &'a str,
    group: PathBuf,
    pairs: Vec<(&'a Path, &'a Path)>,
    out_path: PathBuf,
}

/// One job per directory the widget's drifting frames were captured into.
fn comparison_jobs<'a>(report: &'a WidgetReport, output_dir: &Path) -> Vec<ComparisonJob<'a>> {
    let mut groups: std::collections::BTreeMap<&Path, Vec<(&Path, &Path)>> =
        std::collections::BTreeMap::new();
    for result in &report.results {
        if result.status != "diff" {
            continue;
        }
        let (Some(group), Some(baseline), Some(diff)) = (
            frame_group(&result.rel),
            result.baseline_file.as_deref(),
            result.diff_file.as_deref(),
        ) else {
            continue;
        };
        groups.entry(group).or_default().push((baseline, diff));
    }

    let out_dir = output_dir.join(&report.widget);
    groups
        .into_iter()
        .map(|(group, mut pairs)| {
            pairs.sort_by_key(|(_, diff)| *diff);
            let ext = if pairs.len() == 1 { "png" } else { "mp4" };
            let out_path = super::media::Media::Comparison.path(&out_dir, group, ext);
            ComparisonJob {
                widget: &report.widget,
                group: group.to_owned(),
                pairs,
                out_path,
            }
        })
        .collect()
}

/// Render one group's A/B media: a still for a lone frame, a video for a run.
fn render_comparison(ffmpeg: &Path, job: &ComparisonJob<'_>) -> Result<()> {
    let stack = Stack::for_frame(common_frame_size(&job.pairs)?);
    match job.pairs.as_slice() {
        [(baseline, diff)] => render_comparison_image(ffmpeg, baseline, diff, stack, &job.out_path),
        pairs => render_comparison_video(ffmpeg, pairs, stack, &job.out_path),
    }
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

    let jobs: Vec<_> = failed_reports
        .iter()
        .flat_map(|report| comparison_jobs(report, output_dir))
        .collect();

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
            render_comparison(&ffmpeg_bin, job)
                .err()
                .map(|e| format!("{} {}: {e:#}", job.widget, job.group.display()))
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
        bail!("comparison failures:\n{}", errors.join("\n"));
    }
    Ok(())
}

/// Render a single A/B composite PNG (baseline + diff).
fn render_comparison_image(
    ffmpeg: &Path,
    baseline: &Path,
    diff: &Path,
    stack: Stack,
    output: &Path,
) -> Result<()> {
    let filter = format!("[0][1]{}", stack.filter());
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

/// Render an A/B comparison video from a group's pairs in one ffmpeg call.
///
/// Instead of spawning N+1 processes (composite each frame, then concat+encode),
/// we build one filtergraph that composites every pair, holds each still for 0.5 s,
/// concatenates the streams, and encodes to H.264 — one fork total.
/// 30 fps × 0.5 s = 15 frames per still.
const VIDEO_FPS: u32 = 30;
const VIDEO_HOLD_FRAMES: u32 = 15;

fn render_comparison_video(
    ffmpeg: &Path,
    pairs: &[(&Path, &Path)],
    stack: Stack,
    output: &Path,
) -> Result<()> {
    use std::fmt::Write;

    let stack = stack.filter();

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
    for (baseline, diff) in pairs {
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
    let total =
        report.passed + report.tolerated + report.failed + report.missing + report.new_count;
    let ok =
        !report.no_baseline && report.failed == 0 && report.missing == 0 && report.new_count == 0;

    let tolerated_note = if report.tolerated > 0 {
        format!(
            "  {} tolerated ({})",
            report.tolerated,
            diff_totals(report, "tolerated")
        )
    } else {
        String::new()
    };

    let (mark, right_text) = if report.no_baseline {
        ("\u{2717}", "no baseline".to_owned())
    } else if !ok {
        let mut parts = Vec::new();
        if report.failed > 0 {
            parts.push(format!(
                "{}Δ {}",
                report.failed,
                diff_totals(report, "diff")
            ));
        }
        if report.missing > 0 {
            parts.push(format!("{} missing", report.missing));
        }
        if report.new_count > 0 {
            parts.push(format!("{} new", report.new_count));
        }
        ("\u{2717}", format!("{}{tolerated_note}", parts.join(", ")))
    } else {
        ("\u{2713}", format!("{total}{tolerated_note}"))
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
    let visible_right = 1 + visible_len(&right_text) + 3 + time_str.len();
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
            "  {label} {} {} · {}",
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
            "  {label} {} {} · {}",
            "·".repeat(dots).dimmed(),
            right_text.red(),
            time_str.dimmed()
        )
    }
}

/// An intentional restyle drifts every frame; listing them all would bury the log.
const MAX_FRAME_LINES: usize = 10;

/// Pixel totals across the frames of one status, so a tolerated frame's drift
/// is never counted into the failure summary.
fn diff_totals(report: &WidgetReport, status: &str) -> String {
    let diffs: Vec<PixelDiff> = report
        .results
        .iter()
        .filter(|r| r.status == status)
        .filter_map(|r| r.diff)
        .collect();
    let worst = diffs
        .iter()
        .fold(0.0_f64, |worst, diff| worst.max(diff.percentage));
    let total: usize = diffs.iter().map(|diff| diff.count).sum();
    if worst >= 0.01 {
        format!("{total} px total (↑{worst:.2}% worst)")
    } else {
        format!("{total} px total")
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
            parts.push(format!(
                "Δ {} {}",
                report.failed,
                diff_totals(report, "diff")
            ));
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

        let diffs: Vec<_> = report
            .results
            .iter()
            .filter_map(|result| result.diff.map(|diff| (result, diff)))
            .collect();
        for (result, diff) in diffs.iter().take(MAX_FRAME_LINES) {
            let detail = format!("{} px ({:.2}%)", diff.count, diff.percentage);
            lines.push(format!("      {} {}", result.rel.dimmed(), detail.dimmed()));
        }
        if let Some(rest) = diffs
            .len()
            .checked_sub(MAX_FRAME_LINES)
            .filter(|remaining| *remaining > 0)
        {
            lines.push(format!("      {}", format!("… and {rest} more").dimmed()));
        }
    }
    if !lines.is_empty() {
        eprintln!();
        for line in &lines {
            eprintln!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with_diff(diff: PixelDiff) -> WidgetReport {
        WidgetReport {
            workspace: "widgets".to_owned(),
            widget: "clock".to_owned(),
            results: vec![DiffResult {
                rel: "bmc100/full/bmc100-full/frame_0000.png".to_owned(),
                status: "diff".to_owned(),
                diff: Some(diff),
                ..Default::default()
            }],
            failed: 1,
            ..Default::default()
        }
    }

    #[test]
    fn odiff_match_needs_no_pixel_measurements() {
        let result = parse_odiff_response(r#"{"match":true}"#)
            .expect("a matching odiff response should parse");

        assert!(result.matched);
        assert!(result.diff.is_none());
    }

    #[test]
    fn odiff_pixel_diff_keeps_both_measurements() {
        let result = parse_odiff_response(
            r#"{"match":false,"reason":"pixel-diff","diffCount":17,"diffPercentage":0.01}"#,
        )
        .expect("a complete pixel-diff response should parse");

        assert!(!result.matched);
        let diff = result
            .diff
            .expect("a pixel mismatch should carry its measurements");
        assert_eq!(diff.count, 17);
        assert!(
            (diff.percentage - 0.01).abs() < f64::EPSILON,
            "unexpected percentage: {}",
            diff.percentage
        );
    }

    #[test]
    fn odiff_pixel_diff_rejects_missing_count() {
        let Err(error) =
            parse_odiff_response(r#"{"match":false,"reason":"pixel-diff","diffPercentage":0.01}"#)
        else {
            panic!("a pixel mismatch without a count must be rejected");
        };

        assert!(
            error
                .to_string()
                .contains("mismatch without a usable diffCount"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn odiff_layout_diff_reports_the_pinned_option_violation() {
        let Err(error) = parse_odiff_response(r#"{"match":false,"reason":"layout-diff"}"#) else {
            panic!("failOnLayoutDiff=false should prevent layout-only responses");
        };

        assert!(
            error.to_string().contains("unexpected layout-diff"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn diff_totals_omits_a_percentage_that_odiff_rounded_to_zero() {
        let report = report_with_diff(PixelDiff {
            count: 3,
            percentage: 0.0,
        });

        assert_eq!(diff_totals(&report, "diff"), "3 px total");
    }

    #[test]
    fn diff_totals_leads_with_pixels_before_a_visible_percentage() {
        let report = report_with_diff(PixelDiff {
            count: 900,
            percentage: 0.43,
        });

        assert_eq!(diff_totals(&report, "diff"), "900 px total (↑0.43% worst)");
    }

    #[test]
    fn status_line_separates_nested_drift_summary_from_elapsed_time() {
        let mut report = report_with_diff(PixelDiff {
            count: 900,
            percentage: 0.43,
        });
        report.missing = 3;

        let styled_line = widget_status_line(&report, 0.9);
        let line = console::strip_ansi_codes(&styled_line);

        assert!(
            line.ends_with("1Δ 900 px total (↑0.43% worst), 3 missing · 0.9s"),
            "unexpected status line: {line}"
        );
    }

    #[test]
    fn html_report_shows_tiny_pixel_diff_without_zero_percentage() {
        let report = report_with_diff(PixelDiff {
            count: 3,
            percentage: 0.0,
        });
        let temp = tempfile::tempdir().expect("temporary report directory should be created");
        let output = temp.path().join("report.html");

        generate_html_report(&[report], &output).expect("HTML report should render");
        let html = std::fs::read_to_string(output).expect("HTML report should be readable");

        assert!(html.contains("<span class=\"hint\">3 px</span>"));
        assert!(!html.contains("0.00%"));
    }

    fn pixel_diff(count: usize) -> OdiffResult {
        OdiffResult {
            matched: false,
            diff: Some(PixelDiff {
                count,
                percentage: 0.0,
            }),
        }
    }

    #[test]
    fn a_match_passes_whatever_the_budget() {
        let cmp = OdiffResult {
            matched: true,
            diff: None,
        };
        assert_eq!(verdict(&cmp, 0), Verdict::Pass);
    }

    #[test]
    fn pixels_up_to_the_budget_are_tolerated() {
        assert_eq!(verdict(&pixel_diff(1), 8), Verdict::Tolerated);
        assert_eq!(verdict(&pixel_diff(8), 8), Verdict::Tolerated);
    }

    #[test]
    fn one_pixel_past_the_budget_fails() {
        assert_eq!(verdict(&pixel_diff(9), 8), Verdict::Fail);
    }

    #[test]
    fn a_zero_budget_tolerates_nothing() {
        assert_eq!(verdict(&pixel_diff(1), 0), Verdict::Fail);
    }

    #[test]
    fn tolerated_pixels_stay_out_of_the_failure_totals() {
        let mut report = report_with_diff(PixelDiff {
            count: 900,
            percentage: 0.43,
        });
        report.results.push(DiffResult {
            rel: "bmc100/full/bmc100-full/frame_0001.png".to_owned(),
            status: "tolerated".to_owned(),
            diff: Some(PixelDiff {
                count: 4,
                percentage: 0.0,
            }),
            ..Default::default()
        });
        report.tolerated = 1;

        assert_eq!(diff_totals(&report, "diff"), "900 px total (↑0.43% worst)");
        assert_eq!(diff_totals(&report, "tolerated"), "4 px total");
    }

    // ── Comparison media ─────────────────────────────────────────────

    fn drifted(rel: &str) -> DiffResult {
        DiffResult {
            rel: rel.to_owned(),
            status: "diff".to_owned(),
            baseline_file: Some(PathBuf::from("baseline.png")),
            diff_file: Some(PathBuf::from(rel)),
            ..Default::default()
        }
    }

    fn jobs_for(rels: &[&str]) -> Vec<PathBuf> {
        let report = WidgetReport {
            widget: "clock".to_owned(),
            results: rels.iter().copied().map(drifted).collect(),
            ..Default::default()
        };
        comparison_jobs(&report, Path::new("out"))
            .iter()
            .map(|job| job.out_path.clone())
            .collect()
    }

    #[test]
    fn viewports_of_one_platform_do_not_share_a_comparison() {
        assert_eq!(
            jobs_for(&[
                "bmc100/full/bmc100-full/frame_0000.png",
                "bmc100/small/bmc100-small/frame_0000.png",
            ]),
            [
                PathBuf::from("out/clock/comparison/bmc100-full/bmc100-full.png"),
                PathBuf::from("out/clock/comparison/bmc100-small/bmc100-small.png"),
            ],
        );
    }

    #[test]
    fn datasets_of_one_target_do_not_share_a_comparison() {
        assert_eq!(
            jobs_for(&[
                "bmc100/full/practice/frame_0000.png",
                "bmc100/full/qualifying/frame_0000.png",
            ]),
            [
                PathBuf::from("out/clock/comparison/bmc100-full/practice.png"),
                PathBuf::from("out/clock/comparison/bmc100-full/qualifying.png"),
            ],
        );
    }

    #[test]
    fn a_run_of_frames_becomes_one_video() {
        assert_eq!(
            jobs_for(&[
                "bmc100/full/qualifying/frame_0000.png",
                "bmc100/full/qualifying/frame_0001.png",
            ]),
            [PathBuf::from(
                "out/clock/comparison/bmc100-full/qualifying.mp4"
            )],
        );
    }

    /// Every frame `frame_dir` writes sits three levels down,
    /// so one that does not comes from a corrupt archive.
    #[test]
    fn a_frame_outside_any_directory_makes_no_comparison() {
        assert_eq!(jobs_for(&["frame_0000.png"]), [] as [PathBuf; 0]);
    }

    #[test]
    fn a_wide_frame_stacks_vertically() {
        assert_eq!(Stack::for_frame((1_280, 480)).filter(), "vstack");
    }

    #[test]
    fn a_square_frame_stacks_side_by_side() {
        assert_eq!(Stack::for_frame((480, 480)).filter(), "hstack");
    }

    fn png(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.join(name);
        image::RgbImage::new(width, height)
            .save(&path)
            .expect("BUG: failed to write a test png");
        path
    }

    #[test]
    fn a_group_reports_the_size_its_frames_share() {
        let dir = tempfile::tempdir().expect("BUG: failed to make a temp dir");
        let baseline = png(dir.path(), "baseline.png", 638, 238);
        let diff = png(dir.path(), "diff.png", 638, 238);

        let size = common_frame_size(&[(baseline.as_path(), diff.as_path())])
            .expect("frames of one size have a common size");
        assert_eq!(size, (638, 238));
    }

    #[test]
    fn mismatched_frames_are_named_rather_than_left_to_ffmpeg() {
        let dir = tempfile::tempdir().expect("BUG: failed to make a temp dir");
        let wide = png(dir.path(), "wide.png", 1_280, 480);
        let small = png(dir.path(), "small.png", 317, 238);

        let error = common_frame_size(&[(wide.as_path(), small.as_path())])
            .expect_err("frames of two sizes cannot be stacked");
        let message = error.to_string();
        assert!(
            message.contains("wide.png") && message.contains("small.png"),
            "both frames belong in the error, got: {message}",
        );
    }
}

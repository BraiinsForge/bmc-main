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

//! `verify` subcommand — capture all widgets then diff against baselines.
//!
//! CI entry point: runs `run-all` followed by per-widget `diff`, exits non-zero
//! on any visual regression failures. Generates an HTML report and comparison
//! media for failures.

use std::path::PathBuf;

use anyhow::{Result, bail};
use owo_colors::OwoColorize;
use rayon::prelude::*;

// ── Public interface ────────────────────────────────────────────────

pub struct VerifyArgs {
    pub widget: Option<String>,
    pub threshold: f64,
    pub max_diff_pixels: usize,
    pub html: Option<PathBuf>,
    pub workspaces: Vec<PathBuf>,
    pub wasm_dirs: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub parallel: Option<usize>,
}

pub fn execute(args: &VerifyArgs) -> Result<()> {
    // Print run configuration
    eprintln!();
    let filter = args.widget.as_deref().unwrap_or("all");
    let parallel_str = match args.parallel {
        Some(0) => "auto".to_owned(),
        Some(n) => n.to_string(),
        None => "off".to_owned(),
    };
    eprintln!(
        "{} {} {filter}  {} {parallel_str}  {} {}  {} {}",
        "verify".bold(),
        "widgets:".dimmed(),
        "parallel:".dimmed(),
        "threshold:".dimmed(),
        args.threshold,
        "budget:".dimmed(),
        format_args!("{} px", args.max_diff_pixels),
    );
    eprintln!();

    // Step 1: Capture all (or one) widget
    super::run_all::execute(&super::run_all::RunAllArgs {
        widget: args.widget.clone(),
        workspaces: args.workspaces.clone(),
        wasm_dirs: args.wasm_dirs.clone(),
        output_dir: args.output_dir.clone(),
        parallel: args.parallel,
    })?;

    eprintln!();

    // Step 2: Diff each widget against baselines (parallel — each thread
    // spawns its own odiff server since the server
    // handles one request at a time).
    let widgets =
        super::run_all::resolve_widgets(&args.workspaces, &args.wasm_dirs, args.widget.as_deref())?;

    super::diff::section("Compare");

    let results: Vec<_> = widgets
        .par_iter()
        .map(|entry| {
            let cap_dir = super::run_all::capture_dir(&entry.workspace, &entry.name);
            let widget_output = args.output_dir.join(&entry.name);

            super::diff::execute(&super::diff::DiffArgs {
                workspace: super::run_all::workspace_label(&entry.workspace).to_owned(),
                capture_dir: cap_dir,
                output: widget_output,
                threshold: args.threshold,
                max_diff_pixels: args.max_diff_pixels,
                quiet_progress: widgets.len() > 1,
            })
        })
        .collect();

    let mut reports = Vec::new();
    let mut baseline_dirs = Vec::new();
    let mut has_failures = false;

    // `discover_widgets` sorts alphabetically and rayon's
    // `par_iter().collect()` preserves input order,
    // so Compare prints stably.
    for result in results {
        let (report, baseline_tmp, elapsed) = result?;
        eprintln!("{}", super::diff::widget_status_line(&report, elapsed));
        if report.has_failures() {
            has_failures = true;
        }
        reports.push(report);
        baseline_dirs.extend(baseline_tmp);
    }

    if has_failures && let Err(e) = super::diff::generate_comparisons(&reports, &args.output_dir) {
        eprintln!(
            "\n  {} failed to generate comparison media: {e:#}",
            "warning:".yellow().bold()
        );
    }

    // Generate HTML report
    let html_path = args
        .html
        .clone()
        .unwrap_or_else(|| args.output_dir.join("report.html"));
    super::diff::generate_html_report(&reports, &html_path)?;

    // Drop baseline temp dirs now that comparisons and report are done
    drop(baseline_dirs);

    if has_failures {
        super::diff::print_failure_details(&reports);
        bail!("visual regression failures detected");
    }

    Ok(())
}

// Copyright (C) 2026  Braiins Systems s.r.o.

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
    pub example: Option<String>,
    pub threshold: f64,
    pub html: Option<PathBuf>,
    pub wasm_dir: Option<PathBuf>,
    pub widgets_dir: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub parallel: Option<usize>,
}

pub fn execute(args: &VerifyArgs) -> Result<()> {
    let widgets_dir = args
        .widgets_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("examples"));
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("captures"));

    // Print run configuration
    eprintln!();
    let filter = args.example.as_deref().unwrap_or("all");
    let parallel_str = match args.parallel {
        Some(0) => "auto".to_owned(),
        Some(n) => n.to_string(),
        None => "off".to_owned(),
    };
    eprintln!(
        "{} {} {filter}  {} {parallel_str}  {} {}",
        "verify".bold(),
        "widgets:".dimmed(),
        "parallel:".dimmed(),
        "threshold:".dimmed(),
        args.threshold,
    );
    eprintln!();

    // Step 1: Capture all (or one) widget
    super::run_all::execute(&super::run_all::RunAllArgs {
        example: args.example.clone(),
        wasm_dir: args.wasm_dir.clone(),
        widgets_dir: args.widgets_dir.clone(),
        output_dir: args.output_dir.clone(),
        parallel: args.parallel,
    })?;

    eprintln!();

    // Step 2: Diff each widget against baselines (parallel — each thread
    // spawns its own odiff server since the server handles one request at
    // a time).
    let examples = match &args.example {
        Some(name) => vec![name.clone()],
        None => super::run_all::discover_examples(&widgets_dir)?,
    };

    super::diff::section("Compare");

    let results: Vec<_> = examples
        .par_iter()
        .map(|example| {
            let cap_dir = widgets_dir.join(example).join("capture");
            let widget_output = output_dir.join(example);

            super::diff::execute(&super::diff::DiffArgs {
                capture_dir: cap_dir,
                output: widget_output,
                threshold: args.threshold,
                quiet_progress: examples.len() > 1,
            })
        })
        .collect();

    let mut reports = Vec::new();
    let mut baseline_dirs = Vec::new();
    let mut has_failures = false;

    for result in results {
        let (report, baseline_tmp) = result?;
        if report.has_failures() {
            has_failures = true;
        }
        reports.push(report);
        baseline_dirs.extend(baseline_tmp);
    }

    if has_failures {
        if let Err(e) = super::diff::generate_comparisons(&reports, &output_dir) {
            eprintln!(
                "\n  {} failed to generate comparison media: {e:#}",
                "warning:".yellow().bold()
            );
        }
    }

    // Generate HTML report
    let html_path = args
        .html
        .clone()
        .unwrap_or_else(|| output_dir.join("report.html"));
    super::diff::generate_html_report(&reports, &html_path)?;

    // Drop baseline temp dirs now that comparisons and report are done
    drop(baseline_dirs);

    if has_failures {
        super::diff::print_failure_details(&reports);
        bail!("visual regression failures detected");
    }

    Ok(())
}

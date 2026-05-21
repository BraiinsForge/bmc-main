// Copyright (C) 2026  Braiins Systems s.r.o.

//! Headless screenshot capture for visual regression testing.
//!
//! Renders a single WASM widget at a given resolution, captures frames as PNGs,
//! and exits. Designed for deterministic, reproducible output.

#![expect(clippy::cast_possible_truncation)]

mod diff;
mod preview;
mod run;
mod run_all;
mod set_baseline;
mod tools;
mod verify;

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use owo_colors::OwoColorize;

use bmc_wasm_runtime::capture_config;

// ── CLI ─────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "capture", about = "Visual regression testing toolchain")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Workspaces to discover widgets in, with optional pre-built wasm.
#[derive(Args)]
struct WorkspaceArgs {
    /// Cargo workspace containing widget crates (repeatable).
    #[arg(long, required = true)]
    workspace: Vec<PathBuf>,
    /// Pre-built .wasm dir, one per --workspace (same order). Skips cargo build.
    #[arg(long)]
    wasm_dir: Vec<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Capture a single widget at a given size.
    Run {
        /// Path to the compiled .wasm widget file.
        wasm: PathBuf,
        /// Capture size as WxH (e.g. 1280x480). Required unless --list-variants.
        #[arg(long)]
        size: Option<String>,
        /// Output directory for captured frames. Required unless --list-variants.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Path to a unified fixture file (overrides config lookup).
        #[arg(long)]
        fixture: Option<PathBuf>,
        /// KV variant name to apply.
        #[arg(long)]
        variant: Option<String>,
        /// List available KV variants and exit.
        #[arg(long)]
        list_variants: bool,
        /// Path to the `capture/` directory containing `config.toml` and
        /// fixtures.
        #[arg(long)]
        capture_dir: PathBuf,
    },
    /// Build and capture every widget across the given workspaces (or one widget).
    RunAll {
        /// Capture only this widget (omit to capture all).
        #[arg(long)]
        widget: Option<String>,
        #[command(flatten)]
        ws: WorkspaceArgs,
        /// Root output directory for captures.
        #[arg(long, required = true)]
        output_dir: PathBuf,
        /// Capture widgets in parallel. Optionally specify thread count (default: nproc/2).
        #[arg(long, num_args = 0..=1, default_missing_value = "0")]
        parallel: Option<usize>,
    },
    /// Compare current captures against baselines for a single widget.
    Diff {
        /// Path to the `capture/` directory containing `baselines.7z`.
        #[arg(long)]
        capture_dir: PathBuf,
        /// Path to the widget's output directory (contains `current/`, `diff/`).
        #[arg(long)]
        output: PathBuf,
        /// Color distance threshold for per-pixel comparison.
        /// Small values tolerate minor anti-aliasing and image decode differences
        /// across GPU/Mesa versions (e.g. CI vs local).
        #[arg(long, default_value = "0.1")]
        threshold: f64,
    },
    /// Capture every widget across the given workspaces
    /// and diff against baselines (CI entry point).
    Verify {
        /// Verify only this widget (omit to verify all).
        #[arg(long)]
        widget: Option<String>,
        /// Color distance threshold for per-pixel comparison.
        /// Small values tolerate minor anti-aliasing and image decode
        /// differences across GPU/Mesa versions (e.g. CI vs local).
        #[arg(long, default_value = "0.1")]
        threshold: f64,
        /// Path to write HTML report.
        #[arg(long)]
        html: Option<PathBuf>,
        #[command(flatten)]
        ws: WorkspaceArgs,
        /// Root output directory for captures.
        #[arg(long, required = true)]
        output_dir: PathBuf,
        /// Capture widgets in parallel. Optionally specify thread count (default: nproc/2).
        #[arg(long, num_args = 0..=1, default_missing_value = "0")]
        parallel: Option<usize>,
    },
    /// Generate mp4 preview videos from captured frames.
    Preview {
        /// Path to the widget's output directory (contains `current/`).
        #[arg(long)]
        output: PathBuf,
        /// Only generate previews for this size (e.g. "full").
        #[arg(long)]
        size: Option<String>,
        /// Frame rate for the output video.
        #[arg(long, default_value = "4")]
        fps: u32,
    },
    /// Update baselines.7z from current captures.
    SetBaseline {
        /// Path to the `capture/` directory — baselines.7z is written here.
        #[arg(long)]
        capture_dir: PathBuf,
        /// Path to the widget's output directory (contains `current/`).
        #[arg(long)]
        output: PathBuf,
    },
    /// Generate a default capture/config.toml template.
    Init {
        /// Directory to create the config in (default: current directory).
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
}

// ── Main ────────────────────────────────────────────────────────────

fn main() {
    let result = dispatch();

    // Flush stderr so messages are visible before we exit.
    let _ = std::io::stderr().flush();

    // Hard-exit without running atexit handlers or pthread cleanup.
    // Background threads (mDNS daemon, fetch workers, GL) hold mutexes
    // that crash in glibc's pthread_mutex_lock during std::process::exit.
    #[expect(
        unsafe_code,
        reason = "only safe way to exit with live background threads"
    )]
    unsafe {
        match result {
            Ok(()) => libc::_exit(0),
            Err(e) => {
                print_error(&e);
                libc::_exit(1);
            }
        }
    }
}

/// Pretty-print an anyhow error chain with colors.
fn print_error(err: &anyhow::Error) {
    // Structured config errors get a nice box.
    if let Some(ce) = err.downcast_ref::<capture_config::ConfigError>() {
        eprintln!();
        eprintln!(
            "  {} {}",
            "Invalid capture config".red().bold(),
            ce.path.display().dimmed()
        );
        eprintln!();
        eprintln!("    {}", ce.message);
        if let Some(hint) = &ce.hint {
            eprintln!("    {}", hint.dimmed());
        }
        eprintln!();
        return;
    }

    // Generic fallback for other errors.
    let chain: Vec<_> = err.chain().collect();
    let root = chain.last().expect("BUG: empty error chain");
    eprintln!();
    for line in root.to_string().lines() {
        eprintln!("  {} {line}", "error:".red().bold());
    }
    for cause in &chain[..chain.len().saturating_sub(1)] {
        eprintln!("     {} {cause}", "in".dimmed());
    }
    eprintln!();
}

fn dispatch() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Run {
            wasm,
            size,
            output,
            fixture,
            variant,
            list_variants,
            capture_dir,
        } => {
            tracing_subscriber::fmt::init();
            run::execute(run::RunArgs {
                wasm_path: wasm,
                size,
                output_dir: output,
                fixture,
                variant,
                list_variants,
                capture_dir,
            })
        }
        Command::RunAll {
            widget,
            ws,
            output_dir,
            parallel,
        } => run_all::execute(&run_all::RunAllArgs {
            widget,
            workspaces: ws.workspace,
            wasm_dirs: ws.wasm_dir,
            output_dir,
            parallel,
        }),
        Command::Diff {
            capture_dir,
            output,
            threshold,
        } => {
            let (report, _baseline_tmp, elapsed) = diff::execute(&diff::DiffArgs {
                workspace: String::new(),
                capture_dir,
                output,
                threshold,
                quiet_progress: false,
            })?;
            eprintln!("{}", diff::widget_status_line(&report, elapsed));
            Ok(())
        }
        Command::Verify {
            widget,
            threshold,
            html,
            ws,
            output_dir,
            parallel,
        } => verify::execute(&verify::VerifyArgs {
            widget,
            threshold,
            html,
            workspaces: ws.workspace,
            wasm_dirs: ws.wasm_dir,
            output_dir,
            parallel,
        }),
        Command::Preview { output, size, fps } => {
            preview::execute(&preview::PreviewArgs { output, size, fps })
        }
        Command::SetBaseline {
            capture_dir,
            output,
        } => set_baseline::execute(&set_baseline::SetBaselineArgs {
            capture_dir,
            output,
        }),
        Command::Init { dir } => run::write_default_capture_config(&dir),
    }
}

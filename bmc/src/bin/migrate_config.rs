// Copyright (C) 2025  Braiins Systems s.r.o.

//! Offline config migration tool.
//!
//! Reads a legacy `/etc/bmc_config.json` from `<src>`, translates it
//! to the current schema, and writes the result to `<dst>`. Lets us
//! exercise the translator against captured device samples without
//! flashing firmware.
//!
//! Usage: `bmc-migrate-config <src> <dst>`

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let src: PathBuf = args
        .next()
        .context("usage: bmc-migrate-config <src> <dst>")?
        .into();
    let dst: PathBuf = args
        .next()
        .context("usage: bmc-migrate-config <src> <dst>")?
        .into();
    if args.next().is_some() {
        bail!("usage: bmc-migrate-config <src> <dst>");
    }

    let raw = tokio::fs::read_to_string(&src)
        .await
        .with_context(|| format!("read {}", src.display()))?;
    let report = bmc::config_migration::migrate_raw(&raw, &dst).await?;

    println!(
        "scenes={} translated={} unavailable={} was_legacy={}",
        report.scenes, report.translated_widgets, report.unavailable_widgets, report.was_legacy
    );
    Ok(())
}

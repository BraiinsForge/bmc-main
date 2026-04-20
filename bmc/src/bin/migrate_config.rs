// Copyright (C) 2025  Braiins Systems s.r.o.

//! Offline config migration tool.
//!
//! Reads any-version `/etc/bmc_config.json` from `<src>`, upgrades
//! it in memory to the current schema, and writes the result to
//! `<dst>` (creating a `.backup.<ts>` of `<dst>` if it already
//! existed). Lets us exercise the upgrade path against captured
//! device samples without flashing firmware.
//!
//! Usage: `bmc-migrate-config <src> <dst>`

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use bmc::config_migration::{self, LoadedConfig};

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
    let loaded: LoadedConfig = raw.parse()?;
    config_migration::save_with_backup(loaded.current(), &dst).await?;

    if let Some(report) = loaded.report() {
        println!(
            "scenes={} translated={} dropped={} was_migrated=true",
            report.scenes, report.translated_widgets, report.dropped_widgets,
        );
    } else {
        println!(
            "scenes={} translated=0 dropped=0 was_migrated=false",
            loaded.current().scenes.len(),
        );
    }
    Ok(())
}

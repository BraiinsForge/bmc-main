// Copyright (C) 2025  Braiins Systems s.r.o.

//! Offline config migration tool.
//!
//! Reads any-version BMC config from `<src>`, upgrades it in memory
//! to the current schema, and writes the result to `<dst>` (creating
//! a `.backup.<ts>` of `<dst>` if it already existed). Lets us
//! exercise the upgrade path against captured device samples without
//! flashing firmware.
//!
//! Runtime path on the device is `/etc/bmc/config.json` (moved from
//! the legacy `/etc/bmc_config.json` on first boot of the new
//! firmware); this CLI operates on arbitrary paths so captured
//! samples can live anywhere.
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
            "scenes={} dropped_scenes={} translated_widgets={} dropped_widgets={} was_migrated=true",
            report.scenes, report.dropped_scenes, report.translated_widgets, report.dropped_widgets,
        );
    } else {
        println!(
            "scenes={} dropped_scenes=0 translated_widgets=0 dropped_widgets=0 was_migrated=false",
            loaded.current().scenes().len(),
        );
    }
    Ok(())
}

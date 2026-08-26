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

//! Offline config migration tool.
//!
//! Reads any-version BMC config from `<src>`, upgrades it in memory
//! to the current schema, and writes the result to `<dst>` (creating
//! a `.backup.<ts>` of `<dst>` if it already existed). Lets us
//! exercise the upgrade path against captured device samples without
//! flashing firmware.
//!
//! Runtime path on the device is `/etc/bmc/config.json` (copied from
//! the legacy `/etc/bmc_config.json` on first boot of the new
//! firmware, which keeps the original); this CLI operates on arbitrary
//! paths so captured samples can live anywhere.
//!
//! Usage: `bmc-migrate-config <src> <dst>`

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use bmc::config_migration::{self, LoadedConfig};
use bmc::secret_store::SecretStoreHandle;

// One-shot CLI doing sequential async file I/O — a single-threaded
// runtime is enough; no worker pool needed.
#[tokio::main(flavor = "current_thread")]
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
    // Validate before writing, mirroring the boot path: never bless a
    // config the device would reject and wipe on next boot.
    loaded
        .validate()
        .context("migrated config failed validation; refusing to write it to <dst>")?;
    config_migration::save_with_backup(loaded.current(), &dst).await?;

    // Accounts leave the config for the secret store; mirror the boot
    // path by writing them beside <dst> so the migration loses nothing.
    let extracted = loaded.extracted_accounts();
    let account_count = extracted.len();
    SecretStoreHandle::init(&dst)
        .await
        .merge_extracted(extracted.clone())
        .await
        .context("failed to write the extracted accounts beside <dst>")?;

    // Only the v0 hop produces widget counts;
    // the v1 hop reshapes accounts and widget placement without one,
    // so migrated-ness is read from the load itself.
    let was_migrated = loaded.was_migrated();
    if let Some(report) = loaded.report() {
        println!(
            "scenes={} dropped_scenes={} deactivated_scenes={} translated_widgets={} dropped_widgets={} accounts={account_count} was_migrated={was_migrated}",
            report.scenes,
            report.dropped_scenes,
            report.deactivated_scenes,
            report.translated_widgets,
            report.dropped_widgets,
        );
    } else {
        println!(
            "scenes={} dropped_scenes=0 deactivated_scenes=0 translated_widgets=0 dropped_widgets=0 accounts={account_count} was_migrated={was_migrated}",
            loaded.current().scenes().len(),
        );
    }
    Ok(())
}

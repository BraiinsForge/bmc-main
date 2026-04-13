// Copyright (C) 2025  Braiins Systems s.r.o.

//! Migrate `/etc/bmc_config.json` from the slint-monolith shape to
//! the manifest-driven shape introduced by BDK-141 / BDK-385.
//!
//! Shape-based detection: if the file parses as the current
//! `crate::config::Config`, it's already new and the migrator is a
//! no-op. Otherwise we try the legacy shape, translate, back up the
//! original, and write the result atomically via
//! `crate::utils::replace_file`.
//!
//! See `docs/devlogs/BDK-346/design.md` for the full design.

mod legacy;
mod translate;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tracing::info;

use crate::config::Config;

pub use translate::{MigrationOutcome, Report};

/// Detect and migrate a legacy config in place. Returns a `Report`
/// summarizing what happened (zero-valued if the file was already
/// new).
pub async fn migrate_in_place(path: &Path) -> Result<Report> {
    let raw = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("read config: {}", path.display()))?;
    migrate_raw(&raw, path).await
}

/// Migrate a raw config string. Used by `migrate_in_place` and the
/// `bmc-migrate-config` CLI; exposed separately so tests can drive
/// the full flow without setting up a filesystem fixture.
pub async fn migrate_raw(raw: &str, dest: &Path) -> Result<Report> {
    // Try new format first — cheap, and it covers the already-migrated
    // case on every boot after the first.
    if serde_json::from_str::<Config>(raw).is_ok() {
        return Ok(Report::default());
    }

    let legacy: legacy::Config = serde_json::from_str(raw)
        .context("config parses as neither the current schema nor a recognized legacy schema")?;

    // Always copy the original before touching it. Best-effort: if the
    // dest doesn't exist yet (CLI against an arbitrary path), skip.
    if tokio::fs::try_exists(dest).await.unwrap_or(false) {
        let backup = backup_path_for(dest);
        tokio::fs::copy(dest, &backup)
            .await
            .with_context(|| format!("backup to {}", backup.display()))?;
        info!(backup = %backup.display(), "legacy config detected; backed up original");
    }

    let (migrated_json, report) = translate::translate_config(legacy);
    let rewritten =
        serde_json::to_vec_pretty(&migrated_json).context("serialize migrated config")?;
    crate::utils::replace_file(dest, &rewritten)
        .await
        .with_context(|| format!("write migrated config to {}", dest.display()))?;

    info!(
        translated = report.translated_widgets,
        unavailable = report.unavailable_widgets,
        scenes = report.scenes,
        "config migration complete",
    );

    Ok(report)
}

fn backup_path_for(path: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".backup.{ts}"));
    PathBuf::from(s)
}

// Copyright (C) 2025  Braiins Systems s.r.o.

//! Migrate `/etc/bmc_config.json` from the slint-monolith shape to
//! the manifest-driven shape introduced by BDK-141 / BDK-385.
//!
//! Version-based detection: a top-level `version` field drives the
//! migration path. Missing or `0` means a legacy slint-monolith
//! config that needs translation; `1` means the current schema and
//! the migrator is a no-op; any other value aborts with an explicit
//! error rather than silently overwriting what might be a
//! newer-format config.
//!
//! See `docs/devlogs/BDK-346/design.md` for the full design.

mod legacy;
mod translate;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tracing::{info, warn};

use crate::config::CONFIG_VERSION;

pub use translate::{MigrationOutcome, Report};

/// Minimal view of the on-disk config used only for detecting which
/// migration arm to dispatch to. Parses fast and never fails on fields
/// added in later schema versions.
#[derive(Deserialize)]
struct FormatHeader {
    #[serde(default)]
    version: u32,
}

/// Detect and migrate a legacy config in place. Returns a `Report`
/// summarizing what happened (zero-valued if the file was already
/// at the current version).
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
    let version =
        detect_version(raw).context("config header could not be parsed; file is not valid JSON")?;

    match version {
        0 => migrate_v0_to_current(raw, dest).await,
        CONFIG_VERSION => Ok(Report::noop()),
        other => bail!(
            "unsupported config version: {other}. Refusing to overwrite; a newer firmware may \
             have written this file. Restore a `.backup.<ts>` copy or update the firmware."
        ),
    }
}

fn detect_version(raw: &str) -> Result<u32> {
    let header: FormatHeader = serde_json::from_str(raw)?;
    Ok(header.version)
}

async fn migrate_v0_to_current(raw: &str, dest: &Path) -> Result<Report> {
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
    } else {
        warn!(
            dest = %dest.display(),
            "destination did not exist; skipping backup (CLI or first-time flow)"
        );
    }

    let (migrated_json, report) = translate::translate_config(legacy);
    let rewritten =
        serde_json::to_vec_pretty(&migrated_json).context("serialize migrated config")?;
    crate::utils::replace_file(dest, &rewritten)
        .await
        .with_context(|| format!("write migrated config to {}", dest.display()))?;

    info!(
        translated = report.translated_widgets,
        legacy_remote = report.legacy_remote_widgets,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_version_missing_defaults_to_zero() {
        assert_eq!(
            detect_version(r#"{"scenes": []}"#).expect("BUG: header should parse"),
            0
        );
    }

    #[test]
    fn detect_version_explicit() {
        assert_eq!(
            detect_version(r#"{"version": 1}"#).expect("BUG: header should parse"),
            1
        );
        assert_eq!(
            detect_version(r#"{"version": 2}"#).expect("BUG: header should parse"),
            2
        );
    }

    #[test]
    fn detect_version_rejects_non_json() {
        assert!(detect_version("not json at all").is_err());
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

//! Config migration, typed-per-version style.
//!
//! Adapted from `bos-main/open/bosminer/bosminer-config`, which
//! handled four major schema versions by representing each as its
//! own Rust type linked through an `Upgrade` trait chain. Parsing
//! any version and walking to the latest becomes a sequence of
//! trait method calls the compiler enforces.
//!
//! Key properties:
//!
//! - **Pure, in-memory upgrades.** `LoadedConfig::from_str` never
//!   touches the filesystem. The caller decides whether and when
//!   to persist the result.
//! - **Downgrade-safe.** A config that names a schema version this
//!   binary does not understand is refused rather than rewritten,
//!   so an accidental firmware downgrade cannot silently clobber a
//!   newer config.
//!
//! See `docs/stories/config-migration.md` for user-facing behaviour
//! and `docs/devlogs/BDK-346/design.md` for design notes.

mod upgrade_v0;
pub mod v0;

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tracing::info;

use crate::config::{CONFIG_VERSION, Config};

/// Tag a schema type with its numeric version.
pub trait Version {
    const VERSION: u32;
}

/// Total, in-memory upgrade from one schema version to the next.
///
/// The chain is linear: each older type points to exactly one
/// newer type via the `NextVersion` associated type. The latest
/// type does not implement [`Upgrade`].
pub trait Upgrade: Version + Sized {
    type NextVersion: Version;
    fn upgrade_to_next_version(self) -> Self::NextVersion;
}

impl Version for Config {
    const VERSION: u32 = CONFIG_VERSION;
}

/// Minimal view of an on-disk config used to dispatch to the
/// matching parse arm. Deserializes fast and tolerates unknown
/// fields so we never choke on a schema we haven't seen.
#[derive(Deserialize)]
struct FormatHeader {
    #[serde(default)]
    version: u32,
}

/// Counts derived from an upgrade run. Zero-valued when the file
/// was already at the current version.
///
/// The upgraded [`Config`] records only the widgets that survived,
/// not how many were dropped, so the counts are built inside
/// [`upgrade_v0::upgrade_with_report`] while the v0 → current mapping
/// is still in scope.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Scenes that survived the upgrade, i.e. kept at least one widget.
    pub scenes: usize,
    /// Scenes dropped because every widget in them was dropped,
    /// leaving the scene empty.
    pub dropped_scenes: usize,
    /// Widgets that survived the upgrade, each mapped to the
    /// `widget_type_id` of a shipped `widgets-wasm` manifest.
    pub translated_widgets: usize,
    /// Widgets dropped because their v0 `kind` or `remote_widget` URL
    /// had no native equivalent in the current schema. A `warn!` is
    /// emitted per drop.
    pub dropped_widgets: usize,
}

/// Result of parsing a raw config of unknown version.
///
/// Either the file was already at the current schema, or it was a
/// v0 config whose parse and upgrade both succeeded — in which case
/// the upgraded `Config` and its migration [`Report`] remain in
/// memory.
#[derive(Debug)]
pub enum LoadedConfig {
    /// File already carried `version = CONFIG_VERSION`.
    AlreadyCurrent(Config),
    /// File was a v0 (legacy) config; the upgrade has been applied.
    MigratedFromV0 { current: Config, report: Report },
}

impl LoadedConfig {
    /// Borrow the effective current config, regardless of origin.
    #[must_use]
    pub fn current(&self) -> &Config {
        match self {
            Self::AlreadyCurrent(c) => c,
            Self::MigratedFromV0 { current, .. } => current,
        }
    }

    /// Take ownership of the current config, discarding the
    /// migration report if any.
    #[must_use]
    pub fn into_current(self) -> Config {
        match self {
            Self::AlreadyCurrent(c) => c,
            Self::MigratedFromV0 { current, .. } => current,
        }
    }

    #[must_use]
    pub fn was_migrated(&self) -> bool {
        matches!(self, Self::MigratedFromV0 { .. })
    }

    #[must_use]
    pub fn report(&self) -> Option<&Report> {
        match self {
            Self::AlreadyCurrent(_) => None,
            Self::MigratedFromV0 { report, .. } => Some(report),
        }
    }
}

impl FromStr for LoadedConfig {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let header: FormatHeader = serde_json::from_str(raw)
            .context("config header could not be parsed; file is not valid JSON")?;
        match header.version {
            v if v == Config::VERSION => {
                let current: Config = serde_json::from_str(raw).context(
                    "config header names current schema but body failed to parse as current",
                )?;
                Ok(Self::AlreadyCurrent(current))
            }
            0 => {
                let legacy: v0::Config = serde_json::from_str(raw).context(
                    "config parses as neither the current schema nor a recognized legacy schema",
                )?;
                let (current, report) = upgrade_v0::upgrade_with_report(legacy);
                Ok(Self::MigratedFromV0 { current, report })
            }
            other => bail!(
                "unsupported config version: {other}. Refusing to read; a newer firmware may \
                 have written this file. Restore a `.backup.<ts>` copy or update the firmware."
            ),
        }
    }
}

/// Best-effort read of the `version` field from a raw config without
/// committing to a full parse. `None` if the text is not even valid
/// JSON. The boot path uses this to tell a genuinely corrupt config
/// (safe to replace after backing it up) apart from a readable config
/// whose version is newer than this firmware understands (which must
/// be preserved, never clobbered — see the downgrade-refusal story).
#[must_use]
pub fn peek_version(raw: &str) -> Option<u32> {
    serde_json::from_str::<FormatHeader>(raw)
        .ok()
        .map(|header| header.version)
}

/// Read a config from disk and upgrade it to the current schema in
/// memory. No disk writes other than the one-time legacy-path copy
/// from `/etc/bmc_config.json` → `/etc/bmc/config.json` (see
/// [`relocate_legacy_config_if_present`]; the legacy file is kept, not
/// moved). Pair with [`save_with_backup`] to persist the upgrade.
pub async fn load_any_version(path: &Path) -> Result<LoadedConfig> {
    relocate_legacy_config_if_present(path).await?;
    let raw = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("read config: {}", path.display()))?;
    raw.parse()
}

/// If `path` points at the canonical `/etc/bmc/<something>` layout
/// but does not yet exist, check for a legacy sibling at
/// `/etc/<something>` and copy it in. Silently no-op in any other
/// case (tests with tmp paths, fresh installs, devices that already
/// have the new path).
///
/// **Copy, not move** — the legacy file is left intact so a device
/// that boots older firmware (for debugging or a forced rollback)
/// still finds its config at the legacy path. That snapshot goes
/// stale the moment the new firmware writes an edit, but "boot old
/// firmware with the config it had at upgrade time" stays possible
/// indefinitely at the cost of one redundant on-disk copy.
///
/// Matches a pattern, not a hardcoded path: any `<parent>/bmc/<name>`
/// target looks for `<parent>/bmc_<name>` as its legacy sibling. This
/// keeps the function useful in tests using tmp dirs while avoiding
/// false positives on unrelated paths.
async fn relocate_legacy_config_if_present(path: &Path) -> Result<()> {
    let Some(legacy) = legacy_sibling_for(path) else {
        return Ok(());
    };
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        return Ok(());
    }
    if !tokio::fs::try_exists(&legacy).await.unwrap_or(false) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create parent dir {}", parent.display()))?;
    }
    tokio::fs::copy(&legacy, path)
        .await
        .with_context(|| format!("copy {} → {}", legacy.display(), path.display()))?;
    info!(
        from = %legacy.display(),
        to = %path.display(),
        "copied legacy config to /etc/bmc/ layout (legacy file kept for downgrade safety)"
    );
    Ok(())
}

/// Given a target path shaped like `<parent>/bmc/<name>`, return the
/// legacy sibling `<parent>/bmc_<name>`. None for any other shape.
fn legacy_sibling_for(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let bmc_dir = path.parent()?;
    if bmc_dir.file_name()?.to_str()? != "bmc" {
        return None;
    }
    let grandparent = bmc_dir.parent()?;
    Some(grandparent.join(format!("bmc_{file_name}")))
}

/// Write `config` to `path`, first copying any existing file at
/// that path to a timestamped backup. Safe to call on every save:
/// the backup is only written when there is a file to back up.
pub async fn save_with_backup(config: &Config, path: &Path) -> Result<()> {
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        let backup = backup_path_for(path);
        tokio::fs::copy(path, &backup)
            .await
            .with_context(|| format!("backup to {}", backup.display()))?;
        info!(backup = %backup.display(), "backed up existing config before save");
    }

    let bytes = serde_json::to_vec_pretty(config).context("serialize config")?;
    crate::utils::replace_file(path, &bytes)
        .await
        .with_context(|| format!("write config to {}", path.display()))?;
    Ok(())
}

/// Convenience: load, upgrade if needed, persist if upgraded.
///
/// Used by the boot sequence and by `bmc-migrate-config`. The
/// returned [`LoadedConfig`] lets the caller inspect the migration
/// [`Report`] after persistence has happened.
pub async fn migrate_on_disk(path: &Path) -> Result<LoadedConfig> {
    let loaded = load_any_version(path).await?;
    if let LoadedConfig::MigratedFromV0 { current, report } = &loaded {
        info!(
            scenes = report.scenes,
            dropped_scenes = report.dropped_scenes,
            translated_widgets = report.translated_widgets,
            dropped_widgets = report.dropped_widgets,
            "upgrading legacy config on disk",
        );
        // Validate the upgraded config in memory *before* writing it, so
        // a migration that produces an invalid config never overwrites
        // the readable original on disk — only a config proven valid is
        // persisted. The original is then left intact for a fixed
        // firmware (or manual recovery) rather than replaced.
        current.validate().context(
            "migrated config failed validation; leaving the original config on disk untouched",
        )?;
        save_with_backup(current, path).await?;
    }
    Ok(loaded)
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
    fn current_version_parses_as_already_current() {
        // Scenes and accounts serialize as JSON arrays; see
        // `crate::scene::deserialize_scenes` and
        // `bmc_display::data::deserialize_accounts`.
        let raw = format!(
            r#"{{"version":{},"scenes":[],"accounts":[]}}"#,
            Config::VERSION
        );
        let loaded: LoadedConfig = raw
            .parse()
            .expect("BUG: current-version parse must succeed");
        assert!(!loaded.was_migrated());
        assert!(loaded.report().is_none());
    }

    #[test]
    fn unknown_future_version_is_rejected() {
        let err = r#"{"version":999,"scenes":{}}"#
            .parse::<LoadedConfig>()
            .expect_err("future version must be refused");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("unsupported config version"),
            "error should name the failure mode (got: {msg})",
        );
    }

    #[test]
    fn empty_legacy_parses_and_upgrades() {
        let raw = r#"{"scenes":[],"accounts":[]}"#;
        let loaded: LoadedConfig = raw.parse().expect("BUG: legacy parse must succeed");
        assert!(loaded.was_migrated());
        assert_eq!(loaded.current().version, Config::VERSION);
        let report = loaded
            .report()
            .expect("BUG: migrated load must carry a report");
        assert_eq!(report.scenes, 0);
    }

    #[test]
    fn missing_version_field_is_treated_as_v0() {
        let raw = r#"{"scenes":[]}"#;
        let loaded: LoadedConfig = raw.parse().expect("BUG: missing version must parse as v0");
        assert!(loaded.was_migrated());
    }
}

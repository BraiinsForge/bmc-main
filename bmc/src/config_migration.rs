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
//! - **The original parse survives.** When a load walks through
//!   [`v0::Config`], the parsed v0 struct is preserved inside
//!   [`LoadedConfig::MigratedFromV0`] — callers can inspect it for
//!   debug views, rollback UIs, or CI snapshot checks without
//!   re-reading the disk (which by then may have been rewritten).
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
/// Populated inside [`upgrade_v0::upgrade_with_report`] as each v0
/// widget is dispatched; the distinction between "survived" and
/// "dropped" is not recoverable from the upgraded [`Config`] alone
/// (all surviving widgets carry real UIDs indistinguishable from
/// the `Default::default()` ones), so we build the counts while
/// the v0 → current mapping is still in scope.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Scenes in the source (unchanged by the upgrade).
    pub scenes: usize,
    /// Widgets that survived the upgrade with a reserved
    /// `widget_type_id`. Includes both deep-translated widgets
    /// (e.g. digital-clock) and pass-through widgets whose params
    /// are handed unchanged to a future manifest.
    pub translated_widgets: usize,
    /// Widgets dropped because their v0 `kind` or `remote_widget`
    /// URL did not match any reserved UID in the current schema.
    /// A `warn!` is emitted per drop.
    pub dropped_widgets: usize,
}

/// Result of parsing a raw config of unknown version.
///
/// Either the file was already at the current schema, or it was a
/// v0 config whose parse and upgrade both succeeded — in which
/// case both the pre-upgrade `v0::Config` and the upgraded
/// `Config` remain in memory.
#[derive(Debug)]
pub enum LoadedConfig {
    /// File already carried `version = CONFIG_VERSION`.
    AlreadyCurrent(Config),
    /// File was a v0 (legacy) config; the upgrade has been applied.
    MigratedFromV0 {
        current: Config,
        /// The original v0 struct, preserved for debug views,
        /// rollback, or CI snapshot checks without disk re-reads.
        original: Box<v0::Config>,
        report: Report,
    },
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

    /// Take ownership of the current config, dropping the
    /// preserved original if any.
    #[must_use]
    pub fn into_current(self) -> Config {
        match self {
            Self::AlreadyCurrent(c) => c,
            Self::MigratedFromV0 { current, .. } => current,
        }
    }

    /// The pre-upgrade v0 struct, available iff the load walked
    /// through a v0 parse. `None` when the file was already
    /// current.
    #[must_use]
    pub fn original_v0(&self) -> Option<&v0::Config> {
        match self {
            Self::AlreadyCurrent(_) => None,
            Self::MigratedFromV0 { original, .. } => Some(original),
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
                let original = Box::new(legacy.clone());
                let (current, report) = upgrade_v0::upgrade_with_report(legacy);
                Ok(Self::MigratedFromV0 {
                    current,
                    original,
                    report,
                })
            }
            other => bail!(
                "unsupported config version: {other}. Refusing to read; a newer firmware may \
                 have written this file. Restore a `.backup.<ts>` copy or update the firmware."
            ),
        }
    }
}

/// Read a config from disk and upgrade it to the current schema in
/// memory. No disk writes. Pair with [`save_with_backup`] to
/// persist the upgrade.
pub async fn load_any_version(path: &Path) -> Result<LoadedConfig> {
    let raw = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("read config: {}", path.display()))?;
    raw.parse()
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
/// returned [`LoadedConfig`] lets the caller inspect the original
/// parse after persistence has happened; nothing else re-reads the
/// file.
pub async fn migrate_on_disk(path: &Path) -> Result<LoadedConfig> {
    let loaded = load_any_version(path).await?;
    if let LoadedConfig::MigratedFromV0 {
        current, report, ..
    } = &loaded
    {
        info!(
            scenes = report.scenes,
            translated = report.translated_widgets,
            dropped = report.dropped_widgets,
            "upgrading legacy config on disk",
        );
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
        assert!(loaded.original_v0().is_none());
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
        assert!(loaded.original_v0().is_some());
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

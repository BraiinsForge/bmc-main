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

//! Config migration, typed-per-version style.
//!
//! Adapted from `bos-main/open/bosminer/bosminer-config`, which represents
//! each on-disk schema version as its own Rust type.
//!
//! [`LoadedConfig::from_str`] reads the `version` header, dispatches
//! to the matching parser, and upgrades to the current schema in memory.
//!
//! Each older version has its own parse arm: v0 (slint-monolith)
//! via [`upgrade_v0`], v1 via [`upgrade_v1`]. Both land on
//! the current schema; a later version adds one more arm.
//!
//! Key properties:
//!
//! - **Pure, in-memory upgrades.** `LoadedConfig::from_str` never touches the filesystem.
//!   The caller decides whether and when to persist the result; the boot path keeps
//!   the migrated config in memory and writes it back only on the first genuine change.
//! - **No boot-time rewrite.** Because the boot path does not persist, the on-disk file
//!   keeps its version until a genuine change is saved.
//!   Downgrades are not supported, so a config a newer BMC application wrote is treated
//!   as unreadable (backed up and replaced with defaults) rather than preserved in place.
//!
//! See `docs/stories/config-migration.md` for user-facing behaviour
//! and `docs/devlogs/BDK-346/design.md` for design notes.

mod upgrade_v0;
mod upgrade_v1;
pub mod v0;

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use indexmap::IndexMap;
use serde::Deserialize;
use tracing::info;

pub use crate::config::CONFIG_VERSION;
use crate::config::Config;
use crate::data::{Account, AccountId};

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
    /// File was a v0 (slint-monolith) config; the widget upgrade has been applied and the
    /// reshaped accounts extracted for the secret store.
    MigratedFromV0 {
        current: Config,
        report: Report,
        accounts: IndexMap<AccountId, Account>,
    },
    /// File was a v1 config; the accounts have been reshaped and extracted for the secret store.
    MigratedFromV1 {
        current: Config,
        accounts: IndexMap<AccountId, Account>,
    },
}

impl LoadedConfig {
    /// Borrow the effective current config, regardless of origin.
    #[must_use]
    pub fn current(&self) -> &Config {
        match self {
            Self::AlreadyCurrent(c) => c,
            Self::MigratedFromV0 { current, .. } | Self::MigratedFromV1 { current, .. } => current,
        }
    }

    /// Accounts a migration extracted for the secret store; empty for a current-schema file,
    /// which by construction carries none.
    #[must_use]
    pub fn extracted_accounts(&self) -> &IndexMap<AccountId, Account> {
        static EMPTY: LazyLock<IndexMap<AccountId, Account>> = LazyLock::new(IndexMap::new);
        match self {
            Self::AlreadyCurrent(_) => &EMPTY,
            Self::MigratedFromV0 { accounts, .. } | Self::MigratedFromV1 { accounts, .. } => {
                accounts
            }
        }
    }

    /// Take ownership of the current config and the extracted accounts,
    /// discarding the migration report if any.
    #[must_use]
    pub fn into_parts(self) -> (Config, IndexMap<AccountId, Account>) {
        match self {
            Self::AlreadyCurrent(c) => (c, IndexMap::new()),
            Self::MigratedFromV0 {
                current, accounts, ..
            }
            | Self::MigratedFromV1 { current, accounts } => (current, accounts),
        }
    }

    #[must_use]
    pub fn was_migrated(&self) -> bool {
        matches!(
            self,
            Self::MigratedFromV0 { .. } | Self::MigratedFromV1 { .. }
        )
    }

    #[must_use]
    pub fn report(&self) -> Option<&Report> {
        match self {
            Self::AlreadyCurrent(_) | Self::MigratedFromV1 { .. } => None,
            Self::MigratedFromV0 { report, .. } => Some(report),
        }
    }

    /// Validate the effective current config against the same rules the
    /// boot path enforces.
    ///
    /// Exposed so the offline `bmc-migrate-config` tool refuses to write
    /// a config the device would reject and wipe on next boot, rather
    /// than silently blessing it.
    pub fn validate(&self) -> Result<()> {
        self.current().validate()
    }
}

impl FromStr for LoadedConfig {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let header: FormatHeader = serde_json::from_str(raw)
            .context("config header could not be parsed; file is not valid JSON")?;
        match header.version {
            v if v == CONFIG_VERSION => {
                let current: Config = serde_json::from_str(raw).context(
                    "config header names current schema but body failed to parse as current",
                )?;
                Ok(Self::AlreadyCurrent(current))
            }
            0 => {
                let legacy: v0::Config = serde_json::from_str(raw).context(
                    "config parses as neither the current schema nor a recognized legacy schema",
                )?;
                let (current, report, raw_accounts) = upgrade_v0::upgrade_with_report(legacy);
                let accounts = upgrade_v1::reshape_and_collect_accounts(raw_accounts);
                Ok(Self::MigratedFromV0 {
                    current,
                    report,
                    accounts,
                })
            }
            1 => {
                let document: serde_json::Value = serde_json::from_str(raw)
                    .context("config header names v1 but body is not valid JSON")?;
                let (current, accounts) = upgrade_v1::upgrade(document)?;
                Ok(Self::MigratedFromV1 { current, accounts })
            }
            other => bail!(
                "unsupported config version: {other}; refusing to read a config written by a \
                 newer BMC application"
            ),
        }
    }
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

/// Copy the file at `path` to a timestamped `.backup.<unix_secs>`
/// sibling, if one exists. No-op when there is nothing to back up.
pub(crate) async fn backup_existing(path: &Path) -> Result<()> {
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        let backup = backup_path_for(path);
        tokio::fs::copy(path, &backup)
            .await
            .with_context(|| format!("backup to {}", backup.display()))?;
        info!(backup = %backup.display(), "backed up existing config before save");
    }
    Ok(())
}

/// Write `config` to `path`, first backing up any existing file to a
/// timestamped copy. Safe to call on every save: the backup is only
/// written when there is a file to back up.
pub async fn save_with_backup(config: &Config, path: &Path) -> Result<()> {
    backup_existing(path).await?;
    let bytes = serde_json::to_vec_pretty(config).context("serialize config")?;
    crate::utils::replace_file(path, &bytes)
        .await
        .with_context(|| format!("write config to {}", path.display()))?;
    Ok(())
}

fn backup_path_for(path: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
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
        let raw = format!(r#"{{"version":{CONFIG_VERSION},"scenes":[],"accounts":[]}}"#);
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
        assert_eq!(loaded.current().version, CONFIG_VERSION);
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

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

//! On-disk store for accounts, kept out of the main config file
//! so support bundles, config backups and debug logs never carry
//! credential material.
//!
//! The file lives beside the config, is written atomically with mode 0600,
//! and is skipped wholesale by the support-bundle walker.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::warn;

use crate::data::{Account, AccountId, deserialize_accounts, serialize_accounts};
use crate::utils::replace_file_with_mode;

pub const SECRETS_FILE_NAME: &str = "secrets.json";

/// Owner-only: the file holds plaintext secrets.
const SECRETS_FILE_MODE: u32 = 0o600;

const STORE_VERSION: u32 = 1;

const CHANNEL_CAPACITY: usize = 16;

/// On-disk shape of the secret store.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecrets {
    version: u32,
    #[serde(
        default,
        serialize_with = "serialize_accounts",
        deserialize_with = "deserialize_accounts"
    )]
    accounts: IndexMap<AccountId, Account>,
}

#[derive(Debug, Clone)]
pub struct SecretStoreHandle {
    path: PathBuf,
    accounts: IndexMap<AccountId, Account>,
    accounts_change: broadcast::Sender<()>,
}

impl SecretStoreHandle {
    /// Load the store beside `config_path`.
    /// An unreadable store is backed up and replaced by an empty one,
    /// so a corrupt file costs the accounts but never blocks boot.
    pub async fn init(config_path: &Path) -> Self {
        let path = config_path.with_file_name(SECRETS_FILE_NAME);
        let accounts = match Self::load(&path).await {
            Ok(accounts) => accounts,
            Err(err) => {
                warn!(?err, "unreadable secret store; starting empty");
                Self::backup_unreadable(&path).await;
                IndexMap::new()
            }
        };
        let (accounts_change, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            path,
            accounts,
            accounts_change,
        }
    }

    /// Fold in accounts a config migration extracted, persisting them.
    /// Reports whether the store now holds them,
    /// so the caller drops the config's copy only once they are safely stored.
    ///
    /// A stored entry wins: the store is the only writer of accounts,
    /// so a same-id entry is never the staler of the two.
    /// Overwriting would let a boot whose config save failed
    /// re-extract a pre-rotation secret over the rotated one.
    pub async fn merge_extracted(
        &mut self,
        extracted: IndexMap<AccountId, Account>,
    ) -> Result<bool> {
        if extracted.is_empty() {
            return Ok(false);
        }
        let known = self.accounts.len();
        for (id, account) in extracted {
            self.accounts.entry(id).or_insert(account);
        }
        if self.accounts.len() != known {
            self.save().await?;
        }
        Ok(true)
    }

    async fn load(path: &Path) -> Result<IndexMap<AccountId, Account>> {
        if !path.exists() {
            return Ok(IndexMap::new());
        }
        let raw = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("read secret store: {}", path.display()))?;
        let stored: StoredSecrets =
            serde_json::from_str(&raw).context("secret store failed to parse")?;
        if stored.version > STORE_VERSION {
            bail!(
                "unsupported secret store version: {}; refusing to read a store written by a \
                 newer BMC application",
                stored.version
            );
        }
        Ok(stored.accounts)
    }

    async fn backup_unreadable(path: &Path) {
        if !path.exists() {
            return;
        }
        let backup_path = path.with_extension("json.bcp");
        match tokio::fs::copy(path, &backup_path).await {
            Ok(_) => warn!(
                "backed up unreadable secret store to {}",
                backup_path.display()
            ),
            Err(err) => warn!(?err, "failed to back up unreadable secret store"),
        }
    }

    #[must_use]
    pub fn accounts(&self) -> &IndexMap<AccountId, Account> {
        &self.accounts
    }

    pub fn accounts_mut(&mut self) -> &mut IndexMap<AccountId, Account> {
        &mut self.accounts
    }

    #[must_use]
    pub fn subscribe_accounts_change(&self) -> broadcast::Receiver<()> {
        self.accounts_change.subscribe()
    }

    pub async fn save(&self) -> Result<()> {
        let stored = StoredSecrets {
            version: STORE_VERSION,
            accounts: self.accounts.clone(),
        };
        let data =
            serde_json::to_string_pretty(&stored).context("failed to serialize secret store")?;
        replace_file_with_mode(&self.path, data.as_bytes(), Some(SECRETS_FILE_MODE))
            .await
            .context("failed to replace secret store file")?;
        let _ = self.accounts_change.send(());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use bmc_field_schema::ParamKey;

    use super::*;

    fn account(name: &str) -> Account {
        Account::new(
            crate::credential::BuiltinType::GenericToken.id().to_owned(),
            name.to_owned(),
            IndexMap::new(),
        )
    }

    fn key(field: &str) -> ParamKey {
        ParamKey::try_new(field.to_owned()).expect("BUG: identifier-shaped field key")
    }

    fn config_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("config.json")
    }

    #[tokio::test]
    async fn saving_accounts_wakes_a_subscriber() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let mut handle = SecretStoreHandle::init(&config_path(&dir)).await;
        let mut changed = handle.subscribe_accounts_change();
        let account = account("pool");
        handle.accounts_mut().insert(account.id.clone(), account);

        handle.save().await.expect("BUG: save must succeed");

        changed
            .try_recv()
            .expect("a successful account save must wake credential refresh");
    }

    #[tokio::test]
    async fn missing_store_starts_empty() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let mut handle = SecretStoreHandle::init(&config_path(&dir)).await;
        assert!(handle.accounts().is_empty());
        assert!(
            !handle
                .merge_extracted(IndexMap::new())
                .await
                .expect("BUG: an empty merge must succeed"),
            "an empty extraction reports nothing written"
        );
        assert!(!dir.path().join(SECRETS_FILE_NAME).exists());
    }

    #[tokio::test]
    async fn extracted_accounts_persist_at_owner_only_mode() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let account = account("pool");
        let extracted = IndexMap::from([(account.id.clone(), account)]);

        let mut handle = SecretStoreHandle::init(&config_path(&dir)).await;
        assert!(
            handle
                .merge_extracted(extracted)
                .await
                .expect("BUG: merge must succeed"),
            "a non-empty extraction reports the write"
        );

        let path = dir.path().join(SECRETS_FILE_NAME);
        let mode = std::fs::metadata(&path)
            .expect("BUG: extraction must create the store")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, SECRETS_FILE_MODE);

        let reloaded = SecretStoreHandle::init(&config_path(&dir)).await;
        assert_eq!(reloaded.accounts().len(), 1);
        assert_eq!(
            reloaded.accounts().values().next().map(|a| a.name.as_str()),
            Some("pool")
        );
    }

    #[tokio::test]
    async fn save_round_trips_through_load() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let mut handle = SecretStoreHandle::init(&config_path(&dir)).await;
        let account = account("weather");
        handle.accounts_mut().insert(account.id.clone(), account);
        handle.save().await.expect("BUG: save must succeed");

        let reloaded = SecretStoreHandle::init(&config_path(&dir)).await;
        assert_eq!(reloaded.accounts().len(), 1);
    }

    #[tokio::test]
    async fn unreadable_store_is_backed_up_and_replaced_empty() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join(SECRETS_FILE_NAME);
        std::fs::write(&path, "{ not json").expect("BUG: write corrupt store");

        let handle = SecretStoreHandle::init(&config_path(&dir)).await;

        assert!(handle.accounts().is_empty());
        assert!(dir.path().join("secrets.json.bcp").exists());
    }

    #[tokio::test]
    async fn newer_store_version_is_refused() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join(SECRETS_FILE_NAME);
        std::fs::write(&path, r#"{ "version": 99, "accounts": [] }"#)
            .expect("BUG: write future store");

        let handle = SecretStoreHandle::init(&config_path(&dir)).await;

        assert!(handle.accounts().is_empty());
        assert!(dir.path().join("secrets.json.bcp").exists());
    }

    #[tokio::test]
    async fn a_re_extraction_does_not_undo_a_rotation() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let legacy = account("original");
        let id = legacy.id.clone();

        // Boot one extracts the account, but the config save that would drop
        // it only warns on failure, so the config keeps its pre-rotation copy.
        let mut handle = SecretStoreHandle::init(&config_path(&dir)).await;
        handle
            .merge_extracted(IndexMap::from([(id.clone(), legacy.clone())]))
            .await
            .expect("BUG: first merge must succeed");

        let mut rotated = legacy.clone();
        rotated
            .field_values
            .insert(key("token"), "rotated".to_owned());
        handle.accounts_mut().insert(id.clone(), rotated);
        handle.save().await.expect("BUG: rotation must persist");

        let mut reloaded = SecretStoreHandle::init(&config_path(&dir)).await;
        assert!(
            reloaded
                .merge_extracted(IndexMap::from([(id.clone(), legacy)]))
                .await
                .expect("BUG: re-run merge must succeed"),
            "the config copy may be dropped whenever the store already holds the account"
        );

        assert_eq!(
            reloaded.accounts()[&id].field_values.get(&key("token")),
            Some(&"rotated".to_owned()),
            "a stale config copy must not overwrite a rotated secret"
        );
    }
}

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

//! Tokens that a service on the device writes to a file,
//! read by bmc on behalf of `local-file-token` accounts
//! and handed to widgets as the account's `token` field.

use std::collections::{BTreeSet, HashMap};
use std::io::{ErrorKind, Read as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::Duration;

use bmc_field_schema::credential::{BuiltinType, FILE_TOKEN_PATH_FIELD};
use indexmap::IndexMap;
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::data::{Account, AccountId};
use crate::secret_store::SecretStoreHandle;

/// A token is a header value: one short line. Anything longer is not one.
const MAX_TOKEN_FILE_BYTES: usize = 4_096;

/// Why the token file cannot be used. Reaches log lines,
/// so it names the fault and never carries file content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TokenFileError {
    #[error("missing")]
    Missing,
    #[error("unreadable")]
    Unreadable,
    #[error("not a regular file")]
    NotRegularFile,
    #[error("oversized")]
    Oversized,
    #[error("empty")]
    Empty,
    #[error("not one line of printable ASCII")]
    NotOneLine,
}

/// Read the token held in `path`, or say why it is unusable.
pub fn read_token(path: &Path) -> Result<String, TokenFileError> {
    let metadata = std::fs::metadata(path).map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            TokenFileError::Missing
        } else {
            TokenFileError::Unreadable
        }
    })?;
    if !metadata.is_file() {
        return Err(TokenFileError::NotRegularFile);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(MAX_TOKEN_FILE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| TokenFileError::Unreadable)?;
    if bytes.len() > MAX_TOKEN_FILE_BYTES {
        return Err(TokenFileError::Oversized);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| TokenFileError::NotOneLine)?;
    let token = text.trim_end();
    if token.is_empty() {
        return Err(TokenFileError::Empty);
    }
    if !token.bytes().all(|byte| (0x21..=0x7E).contains(&byte)) {
        return Err(TokenFileError::NotOneLine);
    }
    Ok(token.to_owned())
}

/// What the last read of every configured path yielded: the usable tokens,
/// and beside them the paths that failed and why. Carries no `Debug`,
/// so the cache itself can never reach a log line or a panic message.
#[derive(Default, PartialEq, Eq)]
pub struct FileTokens {
    tokens: HashMap<PathBuf, String>,
    unusable: HashMap<PathBuf, TokenFileError>,
}

impl FileTokens {
    #[must_use]
    pub fn get(&self, path: &Path) -> Option<&str> {
        self.tokens.get(path).map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn with(path: &str, token: &str) -> Self {
        Self {
            tokens: HashMap::from([(PathBuf::from(path), token.to_owned())]),
            unusable: HashMap::new(),
        }
    }
}

/// How soon a rotated or newly written token reaches widgets.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

const CHANNEL_CAPACITY: usize = 16;

/// Shared handle on the token cache and the change signal beside it.
#[derive(Clone)]
pub struct FileTokenCache {
    tokens: Arc<RwLock<FileTokens>>,
    changed: broadcast::Sender<()>,
}

impl Default for FileTokenCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTokenCache {
    #[must_use]
    pub fn new() -> Self {
        let (changed, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            tokens: Arc::new(RwLock::new(FileTokens::default())),
            changed,
        }
    }

    /// A sync guard: readers hold it for a lookup and never across an await.
    pub fn read(&self) -> RwLockReadGuard<'_, FileTokens> {
        self.tokens.read().expect("BUG: file token lock poisoned")
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    /// Re-read every path and replace the cache; report and broadcast when
    /// anything changed. Paths no longer listed drop out silently: they were
    /// not read, so there is nothing to report.
    pub fn refresh(&self, paths: BTreeSet<PathBuf>) -> bool {
        let mut fresh = FileTokens::default();
        for path in paths {
            match read_token(&path) {
                Ok(token) => {
                    fresh.tokens.insert(path, token);
                }
                Err(reason) => {
                    fresh.unusable.insert(path, reason);
                }
            }
        }

        let mut cached = self.tokens.write().expect("BUG: file token lock poisoned");
        if *cached == fresh {
            return false;
        }
        for (path, token) in &fresh.tokens {
            match cached.tokens.get(path) {
                None => info!(path = %path.display(), "token available"),
                Some(previous) if previous != token => {
                    info!(path = %path.display(), "token changed");
                }
                Some(_) => {}
            }
        }
        for (path, reason) in &fresh.unusable {
            if cached.unusable.get(path) != Some(reason) {
                warn!(path = %path.display(), %reason, "token unusable");
            }
        }
        *cached = fresh;
        drop(cached);
        let _ = self.changed.send(());
        true
    }

    /// Poll the paths of every `local-file-token` account
    /// until the runtime stops.
    /// The store lock is held only to copy the paths out,
    /// and the reads run off the worker: an operator-configured path
    /// on a stalled mount must not park one.
    pub fn spawn_poller(
        self,
        secret_store: Arc<tokio::sync::RwLock<SecretStoreHandle>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(POLL_INTERVAL);
            loop {
                ticks.tick().await;
                let paths = token_paths(secret_store.read().await.accounts());
                let cache = self.clone();
                if let Err(error) = tokio::task::spawn_blocking(move || cache.refresh(paths)).await
                {
                    warn!(%error, "token refresh failed");
                }
            }
        })
    }
}

/// The configured path of every file-backed account,
/// each path once however many accounts name it.
#[must_use]
pub fn token_paths(accounts: &IndexMap<AccountId, Account>) -> BTreeSet<PathBuf> {
    accounts
        .values()
        .filter(|account| account.type_id == BuiltinType::LocalFileToken.id())
        .filter_map(|account| account.field_values.get(FILE_TOKEN_PATH_FIELD))
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::Mutex;

    use tracing_subscriber::fmt;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::prelude::*;

    use super::*;

    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl SharedBuf {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().expect("BUG: log buffer poisoned").clone())
                .expect("BUG: log output not utf8")
        }
    }

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("BUG: log buffer poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuf {
        type Writer = SharedBuf;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("BUG: tempdir")
    }

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("BUG: write token file");
        path
    }

    #[test]
    fn a_one_line_token_is_read_without_its_newline() {
        let d = dir();
        let path = write(&d, "t", b"abc123\n");
        assert_eq!(read_token(&path), Ok("abc123".to_owned()));
    }

    #[test]
    fn a_crlf_ending_is_trimmed_too() {
        let d = dir();
        let path = write(&d, "t", b"abc123\r\n");
        assert_eq!(read_token(&path), Ok("abc123".to_owned()));
    }

    #[test]
    fn a_missing_file_says_so() {
        let d = dir();
        assert_eq!(
            read_token(&d.path().join("nope")),
            Err(TokenFileError::Missing)
        );
    }

    /// A FIFO with no writer blocks `open`; stat-first refuses it without
    /// opening, so one bad path cannot stall the poller for every other.
    #[test]
    fn anything_but_a_regular_file_is_refused_before_it_is_opened() {
        let d = dir();
        assert_eq!(read_token(d.path()), Err(TokenFileError::NotRegularFile));
        let socket = d.path().join("sock");
        let _listener = UnixListener::bind(&socket).expect("BUG: bind test socket");
        assert_eq!(read_token(&socket), Err(TokenFileError::NotRegularFile));
        let fifo = d.path().join("fifo");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())
            .expect("BUG: temp path has no NUL");
        // SAFETY: `c_path` is a valid NUL-terminated path for the call's duration.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert_eq!(read_token(&fifo), Err(TokenFileError::NotRegularFile));
    }

    #[test]
    fn an_empty_or_whitespace_only_file_is_empty() {
        let d = dir();
        assert_eq!(read_token(&write(&d, "e", b"")), Err(TokenFileError::Empty));
        assert_eq!(
            read_token(&write(&d, "w", b" \n\n")),
            Err(TokenFileError::Empty)
        );
    }

    #[test]
    fn a_file_over_the_limit_is_oversized() {
        let d = dir();
        let path = write(&d, "big", &vec![b'a'; MAX_TOKEN_FILE_BYTES + 1]);
        assert_eq!(read_token(&path), Err(TokenFileError::Oversized));
    }

    #[test]
    fn a_second_line_interior_space_or_non_ascii_is_not_one_line() {
        let d = dir();
        let reason = Err(TokenFileError::NotOneLine);
        assert_eq!(read_token(&write(&d, "two", b"abc\ndef\n")), reason);
        assert_eq!(read_token(&write(&d, "space", b"abc def\n")), reason);
        assert_eq!(read_token(&write(&d, "utf8", "tökén\n".as_bytes())), reason);
        assert_eq!(read_token(&write(&d, "bin", b"\xff\xfe\n")), reason);
    }

    #[test]
    fn an_unreadable_file_is_unreadable() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = dir();
        let path = write(&d, "locked", b"abc\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
            .expect("BUG: chmod");
        if std::fs::read(&path).is_ok() {
            // Running as root: permissions do not bite, nothing to prove here.
            return;
        }
        assert_eq!(read_token(&path), Err(TokenFileError::Unreadable));
    }

    fn paths(list: &[&PathBuf]) -> BTreeSet<PathBuf> {
        list.iter().map(|p| (*p).clone()).collect()
    }

    #[test]
    fn the_first_refresh_reads_every_usable_path_and_broadcasts_once() {
        let d = dir();
        let good = write(&d, "good", b"tok-1\n");
        let missing = d.path().join("missing");
        let cache = FileTokenCache::new();
        let mut rx = cache.subscribe();

        assert!(cache.refresh(paths(&[&good, &missing])));

        assert_eq!(cache.read().get(&good), Some("tok-1"));
        assert_eq!(cache.read().get(&missing), None);
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err(), "one change, one broadcast");
    }

    #[test]
    fn a_quiet_tick_changes_nothing_and_stays_silent() {
        let d = dir();
        let good = write(&d, "good", b"tok-1\n");
        let cache = FileTokenCache::new();
        cache.refresh(paths(&[&good]));
        let mut rx = cache.subscribe();

        assert!(!cache.refresh(paths(&[&good])));
        assert!(rx.try_recv().is_err());
    }

    /// A mistyped path and one the writing service has yet to write
    /// both reach the widget as the same missing field;
    /// only the log line tells them apart.
    #[test]
    fn an_unusable_path_is_reported_once_and_again_on_a_new_reason() {
        let d = dir();
        let path = d.path().join("token");
        let cache = FileTokenCache::new();
        let logs = SharedBuf::default();
        let subscriber = tracing_subscriber::registry()
            .with(fmt::layer().with_ansi(false).with_writer(logs.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);
        let reported = || logs.text().matches("token unusable").count();

        assert!(cache.refresh(paths(&[&path])));
        assert_eq!(reported(), 1, "the very first read reports the fault");
        assert!(logs.text().contains("reason=missing"));

        assert!(!cache.refresh(paths(&[&path])));
        assert_eq!(reported(), 1, "an unchanged fault is not repeated");

        std::fs::create_dir(&path).expect("BUG: create directory over the path");
        assert!(cache.refresh(paths(&[&path])));
        assert_eq!(reported(), 2, "a changed reason is reported again");
        assert!(logs.text().contains("reason=not a regular file"));

        std::fs::remove_dir(&path).expect("BUG: remove directory");
        write(&d, "token", b"tok-1\n");
        assert!(cache.refresh(paths(&[&path])));
        assert!(logs.text().contains("token available"));
        assert_eq!(reported(), 2, "recovery adds no fault line");
    }

    /// Boser rotates by writing a temp file and renaming it over the target,
    /// which is what a token change looks like from here.
    #[test]
    fn a_rotation_by_rename_replaces_the_cached_value() {
        let d = dir();
        let target = write(&d, "target", b"tok-1\n");
        let cache = FileTokenCache::new();
        cache.refresh(paths(&[&target]));
        let staged = write(&d, "staged", b"tok-2\n");
        std::fs::rename(&staged, &target).expect("BUG: rename");

        assert!(cache.refresh(paths(&[&target])));
        assert_eq!(cache.read().get(&target), Some("tok-2"));
    }

    #[test]
    fn removal_withdraws_the_token_and_recovery_restores_it() {
        let d = dir();
        let target = write(&d, "target", b"tok-1\n");
        let cache = FileTokenCache::new();
        cache.refresh(paths(&[&target]));

        std::fs::remove_file(&target).expect("BUG: remove");
        assert!(cache.refresh(paths(&[&target])));
        assert_eq!(cache.read().get(&target), None);

        write(&d, "target", b"tok-3\n");
        assert!(cache.refresh(paths(&[&target])));
        assert_eq!(cache.read().get(&target), Some("tok-3"));
    }

    /// An account saved between ticks names a path the previous tick never
    /// saw; the next tick reads it. One dropped from the accounts leaves.
    #[test]
    fn the_path_list_drives_what_is_cached() {
        let d = dir();
        let first = write(&d, "first", b"tok-1\n");
        let second = write(&d, "second", b"tok-2\n");
        let cache = FileTokenCache::new();
        cache.refresh(paths(&[&first]));
        assert_eq!(cache.read().get(&second), None);

        assert!(cache.refresh(paths(&[&first, &second])));
        assert_eq!(cache.read().get(&second), Some("tok-2"));

        assert!(cache.refresh(paths(&[&second])));
        assert_eq!(cache.read().get(&first), None);
        assert_eq!(cache.read().get(&second), Some("tok-2"));
    }

    #[test]
    fn token_paths_lists_only_local_file_token_accounts() {
        use bmc_field_schema::ParamKey;
        let key = ParamKey::try_new(FILE_TOKEN_PATH_FIELD.to_owned()).expect("BUG: key");
        let file = Account::new(
            BuiltinType::LocalFileToken.id().to_owned(),
            "file".to_owned(),
            IndexMap::from([(key.clone(), "/run/a".to_owned())]),
        );
        let plain = Account::new(
            BuiltinType::GenericToken.id().to_owned(),
            "plain".to_owned(),
            IndexMap::from([(key.clone(), "/run/b".to_owned())]),
        );
        // A second account on the same file must not read or report it twice.
        let twin = Account::new(
            BuiltinType::LocalFileToken.id().to_owned(),
            "twin".to_owned(),
            IndexMap::from([(key, "/run/a".to_owned())]),
        );
        let accounts: IndexMap<AccountId, Account> = [file, plain, twin]
            .into_iter()
            .map(|a| (a.id.clone(), a))
            .collect();

        assert_eq!(
            token_paths(&accounts),
            BTreeSet::from([PathBuf::from("/run/a")])
        );
    }

    /// The poller reads at spawn, not after the first interval, and each
    /// tick re-derives the path list, so an account saved between ticks is
    /// read on the next one.
    #[tokio::test(start_paused = true)]
    async fn the_poller_reads_at_spawn_and_follows_the_accounts_on_each_tick() {
        use bmc_field_schema::ParamKey;
        let d = dir();
        let first = write(&d, "first", b"tok-1\n");
        let second = write(&d, "second", b"tok-2\n");
        let key = ParamKey::try_new(FILE_TOKEN_PATH_FIELD.to_owned()).expect("BUG: key");
        let file_account = |name: &str, path: &PathBuf| {
            Account::new(
                BuiltinType::LocalFileToken.id().to_owned(),
                name.to_owned(),
                IndexMap::from([(key.clone(), path.display().to_string())]),
            )
        };
        let mut store = SecretStoreHandle::init(&d.path().join("config.json")).await;
        let account = file_account("first", &first);
        store.accounts_mut().insert(account.id.clone(), account);
        let store = Arc::new(tokio::sync::RwLock::new(store));
        let cache = FileTokenCache::new();
        let mut rx = cache.subscribe();
        let poller = cache.clone().spawn_poller(store.clone());

        rx.recv()
            .await
            .expect("BUG: the first read happens at spawn");
        assert_eq!(cache.read().get(&first), Some("tok-1"));
        assert_eq!(cache.read().get(&second), None);

        let account = file_account("second", &second);
        store
            .write()
            .await
            .accounts_mut()
            .insert(account.id.clone(), account);
        tokio::time::advance(POLL_INTERVAL).await;

        rx.recv()
            .await
            .expect("BUG: the next tick reads the new account");
        assert_eq!(cache.read().get(&second), Some("tok-2"));
        poller.abort();
    }
}

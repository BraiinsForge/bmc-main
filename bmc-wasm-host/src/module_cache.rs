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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Instant;

use bmc_wasm_runtime::WasmWidgetModule;
use sha2::{Digest, Sha256};

pub(crate) type ModuleDigest = [u8; 32];
type ModuleEntries = RefCell<HashMap<ModuleDigest, Weak<CachedModule>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheOutcome {
    Hit,
    Miss,
}

pub(crate) struct ModuleLoad {
    pub(crate) lease: ModuleLease,
    pub(crate) byte_len: usize,
    pub(crate) outcome: CacheOutcome,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ModuleCacheError {
    #[error("read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("compile {}: {error:#}", path.display())]
    Compile { path: PathBuf, error: anyhow::Error },
}

pub(crate) struct ModuleCache {
    state: Rc<ModuleCacheState>,
}

struct ModuleCacheState {
    entries: ModuleEntries,
    compiles: Cell<usize>,
}

struct CachedModule {
    digest: ModuleDigest,
    module: WasmWidgetModule,
    cache: Weak<ModuleCacheState>,
}

pub(crate) struct ModuleLease(Rc<CachedModule>);

impl ModuleCache {
    pub(crate) fn new() -> Self {
        Self {
            state: Rc::new(ModuleCacheState {
                entries: RefCell::new(HashMap::new()),
                compiles: Cell::new(0),
            }),
        }
    }

    pub(crate) fn load(&self, path: &Path) -> Result<ModuleLoad, ModuleCacheError> {
        let wasm_bytes = {
            let started = Instant::now();
            let span = tracing::trace_span!(
                "wasm_module_read",
                path = %path.display(),
                bytes = tracing::field::Empty,
            );
            let _entered = span.enter();
            let result = std::fs::read(path).map_err(|source| ModuleCacheError::Read {
                path: path.to_path_buf(),
                source,
            });
            match &result {
                Ok(bytes) => {
                    span.record("bytes", bytes.len());
                    tracing::trace!(
                        elapsed_us = started.elapsed().as_micros(),
                        "wasm module read"
                    );
                }
                Err(error) => tracing::trace!(
                    elapsed_us = started.elapsed().as_micros(),
                    %error,
                    "wasm module read failed"
                ),
            }
            result?
        };

        let digest: ModuleDigest = {
            let started = Instant::now();
            let span = tracing::trace_span!("wasm_module_hash", bytes = wasm_bytes.len());
            let _entered = span.enter();
            let digest = Sha256::digest(&wasm_bytes).into();
            tracing::trace!(
                elapsed_us = started.elapsed().as_micros(),
                digest = tracing::field::debug(&digest),
                "wasm module hashed"
            );
            digest
        };

        let cached = {
            let started = Instant::now();
            let span = tracing::trace_span!(
                "wasm_module_lookup",
                digest = tracing::field::debug(&digest),
                outcome = tracing::field::Empty,
            );
            let _entered = span.enter();
            let cached = self
                .state
                .entries
                .borrow()
                .get(&digest)
                .and_then(Weak::upgrade);
            let outcome = if cached.is_some() {
                CacheOutcome::Hit
            } else {
                CacheOutcome::Miss
            };
            span.record("outcome", tracing::field::debug(outcome));
            tracing::trace!(
                elapsed_us = started.elapsed().as_micros(),
                "wasm module lookup completed"
            );
            cached
        };

        if let Some(cached) = cached {
            return Ok(ModuleLoad {
                lease: ModuleLease(cached),
                byte_len: wasm_bytes.len(),
                outcome: CacheOutcome::Hit,
            });
        }

        self.state.entries.borrow_mut().remove(&digest);

        let compile_attempt = self.state.compiles.get() + 1;
        self.state.compiles.set(compile_attempt);
        tracing::trace!(compile_attempt, "compiling wasm module cache miss");
        let module =
            WasmWidgetModule::compile(&wasm_bytes).map_err(|error| ModuleCacheError::Compile {
                path: path.to_path_buf(),
                error,
            })?;
        let cached = Rc::new(CachedModule {
            digest,
            module,
            cache: Rc::downgrade(&self.state),
        });
        self.state
            .entries
            .borrow_mut()
            .insert(digest, Rc::downgrade(&cached));

        Ok(ModuleLoad {
            lease: ModuleLease(cached),
            byte_len: wasm_bytes.len(),
            outcome: CacheOutcome::Miss,
        })
    }

    #[cfg(test)]
    pub(crate) fn compile_count(&self) -> usize {
        self.state.compiles.get()
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.state.entries.borrow().len()
    }
}

impl ModuleLease {
    pub(crate) fn module(&self) -> &WasmWidgetModule {
        &self.0.module
    }

    pub(crate) fn digest(&self) -> ModuleDigest {
        self.0.digest
    }

    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Drop for CachedModule {
    fn drop(&mut self) {
        let Some(cache) = self.cache.upgrade() else {
            return;
        };
        let mut entries = cache.entries.borrow_mut();
        let is_current = entries
            .get(&self.digest)
            .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), std::ptr::from_ref(self)));
        if is_current {
            entries.remove(&self.digest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget_bytes(probe: i32) -> Vec<u8> {
        wat::parse_str(format!(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "__bmc_sdk_init") (result i64)
                i64.const {})
              (func (export "render") (param i32))
              (func (export "probe") (result i32)
                i32.const {probe}))
            "#,
            bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
        ))
        .expect("BUG: cache test WAT must parse")
    }

    fn write_widget(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, bytes).expect("BUG: cache test widget must be writable");
        path
    }

    #[test]
    fn identical_bytes_share_one_entry_until_the_final_lease_drops() {
        let directory = tempfile::tempdir().expect("BUG: cache test needs a temporary directory");
        let bytes = widget_bytes(1);
        let first_path = write_widget(directory.path(), "first.wasm", &bytes);
        let second_path = write_widget(directory.path(), "second.wasm", &bytes);
        let cache = ModuleCache::new();

        let first = cache.load(&first_path).expect("valid module must load");
        let same_path = cache
            .load(&first_path)
            .expect("identical module at the same path must load");
        let second_path = cache
            .load(&second_path)
            .expect("identical module at another path must load");

        assert!(
            first.lease.ptr_eq(&same_path.lease),
            "unchanged bytes at one path must share one compiled module"
        );
        assert!(
            first.lease.ptr_eq(&second_path.lease),
            "identical bytes at different paths must share one compiled module"
        );
        assert_eq!(
            cache.compile_count(),
            1,
            "identical bytes must compile once"
        );
        assert_eq!(
            cache.entry_count(),
            1,
            "identical bytes must occupy one registry entry"
        );
        drop(first);
        drop(same_path);
        assert_eq!(
            cache.entry_count(),
            1,
            "a sibling lease must keep the entry live"
        );
        drop(second_path);
        assert_eq!(
            cache.entry_count(),
            0,
            "the final lease must remove the entry immediately"
        );
    }

    #[test]
    fn changed_bytes_at_one_path_coexist_with_the_live_old_module() {
        let directory = tempfile::tempdir().expect("BUG: cache test needs a temporary directory");
        let path = write_widget(directory.path(), "widget.wasm", &widget_bytes(1));
        let cache = ModuleCache::new();
        let old = cache.load(&path).expect("old module must load");

        std::fs::write(&path, widget_bytes(2)).expect("BUG: replacement widget must be writable");
        let new = cache.load(&path).expect("replacement module must load");

        assert_ne!(
            old.lease.digest(),
            new.lease.digest(),
            "changed bytes must produce a new digest"
        );
        assert!(
            !old.lease.ptr_eq(&new.lease),
            "changed bytes must not share a compiled module"
        );
        assert_eq!(
            cache.compile_count(),
            2,
            "each live content version must compile once"
        );
        assert_eq!(
            cache.entry_count(),
            2,
            "old and new content must coexist while both are leased"
        );
        drop(old);
        assert_eq!(
            cache.entry_count(),
            1,
            "dropping the old lease must preserve the new entry"
        );
        drop(new);
        assert_eq!(
            cache.entry_count(),
            0,
            "dropping both versions must empty the registry"
        );
    }

    #[test]
    fn fully_evicted_content_compiles_again() {
        let directory = tempfile::tempdir().expect("BUG: cache test needs a temporary directory");
        let path = write_widget(directory.path(), "widget.wasm", &widget_bytes(1));
        let cache = ModuleCache::new();

        drop(cache.load(&path).expect("valid module must load"));
        assert_eq!(
            cache.entry_count(),
            0,
            "dropping the only lease must evict its entry"
        );
        let reloaded = cache.load(&path).expect("evicted module must reload");

        assert_eq!(
            cache.compile_count(),
            2,
            "an evicted module must compile on its next load"
        );
        drop(reloaded);
    }

    #[test]
    fn read_and_compile_failures_leave_no_entry() {
        let directory = tempfile::tempdir().expect("BUG: cache test needs a temporary directory");
        let cache = ModuleCache::new();
        let missing = directory.path().join("missing.wasm");

        assert!(matches!(
            cache.load(&missing),
            Err(ModuleCacheError::Read { .. })
        ));
        assert_eq!(
            cache.compile_count(),
            0,
            "a read failure must not attempt compilation"
        );
        assert_eq!(
            cache.entry_count(),
            0,
            "a read failure must not create an entry"
        );

        let invalid = write_widget(directory.path(), "invalid.wasm", b"not wasm");
        assert!(matches!(
            cache.load(&invalid),
            Err(ModuleCacheError::Compile { .. })
        ));
        assert_eq!(
            cache.compile_count(),
            1,
            "invalid bytes must count as one compile attempt"
        );
        assert_eq!(
            cache.entry_count(),
            0,
            "a compile failure must not create an entry"
        );
    }

    #[test]
    fn dropping_cache_before_lease_is_a_no_op_for_lease_teardown() {
        let directory = tempfile::tempdir().expect("BUG: cache test needs a temporary directory");
        let path = write_widget(directory.path(), "widget.wasm", &widget_bytes(1));
        let cache = ModuleCache::new();
        let loaded = cache.load(&path).expect("valid module must load");

        drop(cache);
        drop(loaded);
    }

    #[test]
    fn expired_entry_is_replaced_by_a_live_module() {
        let directory = tempfile::tempdir().expect("BUG: cache test needs a temporary directory");
        let bytes = widget_bytes(1);
        let path = write_widget(directory.path(), "widget.wasm", &bytes);
        let digest: ModuleDigest = Sha256::digest(&bytes).into();
        let placeholder = Rc::new(CachedModule {
            digest,
            module: WasmWidgetModule::compile(&bytes).expect("valid placeholder must compile"),
            cache: Weak::new(),
        });
        let expired = Rc::downgrade(&placeholder);
        drop(placeholder);
        let cache = ModuleCache::new();
        cache.state.entries.borrow_mut().insert(digest, expired);

        let loaded = cache.load(&path).expect("expired entry must reload");
        let current = cache
            .state
            .entries
            .borrow()
            .get(&digest)
            .and_then(Weak::upgrade)
            .expect("BUG: successful reload must install a live entry");

        assert_eq!(
            cache.compile_count(),
            1,
            "only the cache miss must count as a compile attempt"
        );
        assert_eq!(
            cache.entry_count(),
            1,
            "the expired entry must be replaced, not retained"
        );
        assert!(
            Rc::ptr_eq(&current, &loaded.lease.0),
            "the registry must point at the returned lease"
        );
    }

    #[test]
    fn expired_entry_is_removed_when_recompilation_fails() {
        let directory = tempfile::tempdir().expect("BUG: cache test needs a temporary directory");
        let invalid_bytes = b"not wasm";
        let path = write_widget(directory.path(), "widget.wasm", invalid_bytes);
        let digest: ModuleDigest = Sha256::digest(invalid_bytes).into();
        let valid_bytes = widget_bytes(1);
        let placeholder = Rc::new(CachedModule {
            digest,
            module: WasmWidgetModule::compile(&valid_bytes)
                .expect("valid placeholder must compile"),
            cache: Weak::new(),
        });
        let expired = Rc::downgrade(&placeholder);
        drop(placeholder);
        let cache = ModuleCache::new();
        cache.state.entries.borrow_mut().insert(digest, expired);

        assert!(matches!(
            cache.load(&path),
            Err(ModuleCacheError::Compile { .. })
        ));
        assert_eq!(
            cache.compile_count(),
            1,
            "only the failed cache compile must be counted"
        );
        assert_eq!(
            cache.entry_count(),
            0,
            "a failed reload must not retain the expired entry"
        );
    }
}

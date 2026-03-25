// Copyright (C) 2026  Braiins Systems s.r.o.

//! Hot-reload support: watches story sources, rebuilds the cdylib, and
//! loads it via `dlopen`.
//!
//! The reload lifecycle is driven by the main thread's frame loop:
//!
//! 1. `notify` watcher detects source changes → sets `source_changed` flag
//! 2. `poll()` sees the flag → spawns `cargo build -p bmc-storybook-stories`
//! 3. Next `poll()` checks if build process exited
//! 4. On success → `try_load_so()` opens the .so and extracts entries
//! 5. On failure → stores cargo stderr for UI display
//!
//! A separate watcher on the .so file supports external build tools
//! (e.g. `cargo-watch`) — any .so change triggers a reload attempt.

use std::fmt;
use std::io;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use std::fs;

use bmc_storybook_api::{StoryEntry, StoryGroupMeta, StoryManifest};

use bmc_storybook_api::knobs::StoryCtx;

/// Debounce delay after .so file change before attempting dlopen.
const SO_DEBOUNCE: Duration = Duration::from_millis(100);

/// Retry delay if dlopen fails (cargo may still be writing).
const SO_RETRY_DELAY: Duration = Duration::from_millis(200);

// ── Owned types (strings cloned from .so before dlclose) ─────────────

/// Story entry with owned strings. Holds an `Arc<Library>` to keep the .so
/// alive while function pointers are in use.
pub struct OwnedStoryEntry {
    pub render_fn: fn(&mut StoryCtx),
    pub name: String,
    pub module_path: String,
    pub source: String,
    pub grid: bool,
    /// When `true` and the story is the only one in its group, the sidebar
    /// collapses the group to a flat entry.
    pub default: bool,
    /// Keeps the .so mapped. `None` for statically-linked stories.
    _library: Option<Arc<libloading::Library>>,
}

impl std::fmt::Debug for OwnedStoryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedStoryEntry")
            .field("name", &self.name)
            .field("module_path", &self.module_path)
            .field("grid", &self.grid)
            .finish_non_exhaustive()
    }
}

impl OwnedStoryEntry {
    /// Create from a statically-linked inventory entry (no .so backing).
    #[must_use]
    pub fn from_static(entry: &StoryEntry) -> Self {
        Self {
            render_fn: entry.render_fn,
            name: entry.name.to_owned(),
            module_path: entry.module_path.to_owned(),
            source: entry.source.to_owned(),
            grid: entry.grid,
            default: entry.default,
            _library: None,
        }
    }

    /// Create from a dynamically-loaded entry, pinning the library alive.
    fn from_dynamic(entry: &StoryEntry, library: &Arc<libloading::Library>) -> Self {
        Self {
            render_fn: entry.render_fn,
            name: entry.name.to_owned(),
            module_path: entry.module_path.to_owned(),
            source: entry.source.to_owned(),
            grid: entry.grid,
            default: entry.default,
            _library: Some(Arc::clone(library)),
        }
    }
}

/// Group metadata with owned strings.
#[derive(Debug, Clone)]
pub struct OwnedStoryGroupMeta {
    pub module_path: String,
    pub title: String,
    pub grid: bool,
}

impl OwnedStoryGroupMeta {
    #[must_use]
    pub fn from_static(meta: &StoryGroupMeta) -> Self {
        Self {
            module_path: meta.module_path.to_owned(),
            title: meta.title.to_owned(),
            grid: meta.grid,
        }
    }
}

// ── Hot reloader ─────────────────────────────────────────────────────

/// Events surfaced by `poll()` for the app to react to.
#[derive(Debug)]
pub enum ReloadEvent {
    /// A cargo build was spawned (show "Building..." in UI).
    BuildStarted,
    /// Build succeeded — call `try_load_so()` to load the new .so.
    BuildSucceeded,
    /// Build failed with captured stderr.
    BuildFailed(String),
    /// The .so file changed externally — call `try_load_so()`.
    SoChanged,
}

/// Internal classification of `.so` load failures so the retry loop can
/// skip permanent ones (ABI / symbol mismatches) without blocking the UI.
#[derive(Debug)]
enum LoadSoError {
    Copy(io::Error),
    Dlopen(libloading::Error),
    SymbolLookup(libloading::Error),
}

impl LoadSoError {
    /// `.so` is structurally complete but missing expected exports — that's
    /// an ABI mismatch (e.g. .so built against an older SDK), retrying after
    /// a sleep won't recover.
    fn is_permanent(&self) -> bool {
        matches!(self, Self::SymbolLookup(_))
    }
}

impl fmt::Display for LoadSoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Copy(e) => write!(f, "failed to copy .so to temp path: {e}"),
            Self::Dlopen(e) => write!(f, "dlopen failed: {e}"),
            Self::SymbolLookup(e) => write!(f, "symbol lookup failed: {e}"),
        }
    }
}

#[expect(missing_debug_implementations)]
pub struct HotReloader {
    so_path: PathBuf,
    library: Option<Arc<libloading::Library>>,
    /// Set by the .so file watcher when the file changes.
    so_changed: Arc<AtomicBool>,
    /// Set by the workspace file watcher when any source file changes.
    source_changed: Arc<AtomicBool>,
    _so_watcher: notify::RecommendedWatcher,
    _source_watcher: notify::RecommendedWatcher,
    /// In-flight cargo build process.
    build_process: Option<Child>,
    /// Debounce: when .so change was first detected.
    so_debounce_start: Option<Instant>,
    /// Monotonic counter for unique temp .so filenames.
    load_generation: u64,
    /// Suppress .so watcher events briefly after a managed build completes,
    /// since `BuildSucceeded` already triggers a load.
    suppress_so_until: Option<Instant>,
    /// When set, a previous `try_load_so` call hit a transient dlopen failure
    /// (cargo still flushing the .so). At this instant `poll()` re-emits
    /// `SoChanged` so the caller retries — keeps the retry off the UI thread.
    load_retry_at: Option<Instant>,
}

impl HotReloader {
    /// Create a new hot reloader.
    ///
    /// - `so_path`: path to `libbmc_storybook_stories.so`
    /// - `workspace_root`: workspace root directory (watched recursively for changes)
    pub fn new(so_path: PathBuf, workspace_root: &Path) -> Result<Self, String> {
        use notify::Watcher;

        // Clean up stale temp files from previous runs (crash recovery).
        cleanup_temp_so_files(&so_path);

        let so_changed = Arc::new(AtomicBool::new(false));
        let source_changed = Arc::new(AtomicBool::new(false));

        // Watch the directory containing the .so
        let flag = Arc::clone(&so_changed);
        let so_file = so_path.clone();
        let mut so_watcher =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only fire for the specific .so file, not other target dir noise.
                    if event.paths.iter().any(|p| p == &so_file)
                        && (event.kind.is_modify() || event.kind.is_create())
                    {
                        flag.store(true, Ordering::Release);
                    }
                }
            })
            .map_err(|e| format!("failed to create .so watcher: {e}"))?;

        if let Some(parent) = so_path.parent() {
            so_watcher
                .watch(parent, notify::RecursiveMode::NonRecursive)
                .map_err(|e| format!("failed to watch .so directory: {e}"))?;
        }

        // Watch source directories to pick up transitive-dep edits (e.g.
        // editing `bmc-keyboard/` while running storybook). The naive
        // approach — `RecursiveMode::Recursive` on the workspace root —
        // tells inotify to install a watch on every subdirectory under
        // `target/` (hundreds of thousands after a debug build), easily
        // hitting `fs.inotify.max_user_watches` (8192 on many distros) and
        // silently breaking the watcher.
        //
        // Instead: walk the workspace root once at startup, recursively
        // watch each non-denylisted top-level directory, and watch the
        // root itself non-recursively for top-level files like `Cargo.toml`
        // and `justfile`. Total watched directories is bounded by source
        // tree size, never by `target/`.
        //
        // The event filter still applies an extension allowlist
        // (`rs`/`png`/`jpg`/`jpeg`/`glb`/`svg`/`toml`) to suppress noise
        // from incidental file activity inside source dirs.
        let flag = Arc::clone(&source_changed);
        let mut source_watcher =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    let has_relevant_ext = event.paths.iter().any(|p| {
                        p.extension().is_some_and(|ext| {
                            matches!(
                                ext.to_str(),
                                Some("rs" | "png" | "jpg" | "jpeg" | "glb" | "svg" | "toml")
                            )
                        })
                    });
                    if has_relevant_ext && (event.kind.is_modify() || event.kind.is_create()) {
                        flag.store(true, Ordering::Release);
                    }
                }
            })
            .map_err(|e| format!("failed to create source watcher: {e}"))?;

        // Top-level files (Cargo.toml, justfile, etc.) without recursing into
        // `target/`.
        source_watcher
            .watch(workspace_root, notify::RecursiveMode::NonRecursive)
            .map_err(|e| format!("failed to watch {}: {e}", workspace_root.display()))?;

        // Each non-denylisted top-level directory, recursively.
        let mut watched_count = 0_usize;
        let entries = std::fs::read_dir(workspace_root)
            .map_err(|e| format!("failed to read workspace root: {e}"))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name_bytes = name.as_encoded_bytes();
            // Denylist: build artifacts, vcs, dotdirs.
            if name_bytes == b"target"
                || name_bytes == b"node_modules"
                || name_bytes.starts_with(b".")
            {
                continue;
            }
            source_watcher
                .watch(&path, notify::RecursiveMode::Recursive)
                .map_err(|e| format!("failed to watch {}: {e}", path.display()))?;
            watched_count += 1;
        }
        tracing::info!(
            root = %workspace_root.display(),
            top_level_dirs = watched_count,
            "hot-reload: watching workspace"
        );

        tracing::info!(so = %so_path.display(), "hot-reload: watchers started");

        Ok(Self {
            so_path,
            library: None,
            so_changed,
            source_changed,
            _so_watcher: so_watcher,
            _source_watcher: source_watcher,
            build_process: None,
            so_debounce_start: None,
            load_generation: 0,
            suppress_so_until: None,
            load_retry_at: None,
        })
    }

    /// Poll for reload events. Call once per frame from the main thread.
    pub fn poll(&mut self) -> Option<ReloadEvent> {
        // 1. Check if an in-flight build has finished.
        if let Some(child) = &mut self.build_process {
            return match child.try_wait() {
                Ok(Some(status)) => {
                    let mut child = self.build_process.take().expect("BUG: just checked");
                    // Capture stderr before reaping.
                    let mut stderr = String::new();
                    if let Some(mut err) = child.stderr.take() {
                        let _ = err.read_to_string(&mut stderr);
                    }
                    // Reap the process to avoid zombies.
                    let _ = child.wait();
                    if status.success() {
                        tracing::info!("hot-reload: build succeeded");
                        // Suppress .so watcher events for 500ms — the build already
                        // produced the .so, so the watcher will fire redundantly.
                        self.suppress_so_until = Some(Instant::now() + Duration::from_millis(500));
                        self.so_debounce_start = None;
                        self.so_changed.store(false, Ordering::Release);
                        return Some(ReloadEvent::BuildSucceeded);
                    }
                    tracing::warn!("hot-reload: build failed");
                    Some(ReloadEvent::BuildFailed(stderr))
                }
                Ok(None) => {
                    // Still building — don't start another build or reload.
                    None
                }
                Err(e) => {
                    self.build_process = None;
                    tracing::error!("hot-reload: failed to wait on build process: {e}");
                    Some(ReloadEvent::BuildFailed(format!(
                        "failed to wait on build: {e}"
                    )))
                }
            };
        }

        // 2. Check if source files changed → start a build.
        if self.source_changed.swap(false, Ordering::AcqRel) {
            tracing::info!("hot-reload: source change detected, starting build");
            return match self.start_build() {
                Ok(()) => Some(ReloadEvent::BuildStarted),
                Err(e) => Some(ReloadEvent::BuildFailed(e)),
            };
        }

        // 3. Check if .so changed externally (e.g. cargo-watch).
        //    Suppressed for a brief window after a managed build completes,
        //    since BuildSucceeded already triggers a load.
        if self.so_changed.swap(false, Ordering::AcqRel) {
            let suppressed = self
                .suppress_so_until
                .is_some_and(|deadline| Instant::now() < deadline);
            if !suppressed {
                self.so_debounce_start = Some(Instant::now());
            }
        }

        // Clear expired suppression.
        if self
            .suppress_so_until
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.suppress_so_until = None;
        }

        // 4. Debounce: wait before attempting dlopen.
        if let Some(start) = self.so_debounce_start
            && start.elapsed() >= SO_DEBOUNCE
        {
            self.so_debounce_start = None;
            return Some(ReloadEvent::SoChanged);
        }

        // 5. Pending dlopen retry from a prior transient failure.
        //    Re-emit `SoChanged` once the retry delay has elapsed so the
        //    caller invokes `try_load_so` again next frame.
        if let Some(deadline) = self.load_retry_at
            && Instant::now() >= deadline
        {
            self.load_retry_at = None;
            return Some(ReloadEvent::SoChanged);
        }

        None
    }

    /// Spawn `cargo build -p bmc-storybook-stories`.
    pub fn start_build(&mut self) -> Result<(), String> {
        if self.build_process.is_some() {
            return Ok(()); // Already building
        }

        // Find workspace root (parent of source_dir's grandparent).
        let workspace_root = self
            .so_path
            .ancestors()
            .find(|p| p.join("Cargo.toml").exists() && p.join("Cargo.lock").exists())
            .unwrap_or(Path::new("."));

        let child = Command::new("cargo")
            .args(["build", "-p", "bmc-storybook-stories", "--color=always"])
            .current_dir(workspace_root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn cargo build: {e}"))?;

        self.build_process = Some(child);
        Ok(())
    }

    /// Is a build currently in progress?
    #[must_use]
    pub fn is_building(&self) -> bool {
        self.build_process.is_some()
    }

    /// Attempt to load (or reload) the .so. Returns owned entries and groups.
    ///
    /// On success, the old Library's `Arc` refcount decreases. If no
    /// `OwnedStoryEntry` from the previous load still exists, the old .so
    /// is `dlclose`d.
    pub fn try_load_so(
        &mut self,
    ) -> Result<(Vec<OwnedStoryEntry>, Vec<OwnedStoryGroupMeta>), String> {
        match self.load_so_inner() {
            Ok(result) => {
                self.load_retry_at = None;
                Ok(result)
            }
            Err(e) if e.is_permanent() => {
                self.load_retry_at = None;
                Err(e.to_string())
            }
            Err(e) => {
                // Transient failure — cargo may still be flushing the .so.
                // Schedule a retry on the next `poll()` after `SO_RETRY_DELAY`
                // instead of sleeping the UI thread.
                tracing::warn!(
                    "hot-reload: dlopen failed, scheduling retry in {SO_RETRY_DELAY:?}: {e}"
                );
                self.load_retry_at = Some(Instant::now() + SO_RETRY_DELAY);
                Err(e.to_string())
            }
        }
    }

    #[expect(unsafe_code)]
    fn load_so_inner(
        &mut self,
    ) -> Result<(Vec<OwnedStoryEntry>, Vec<OwnedStoryGroupMeta>), LoadSoError> {
        // Copy the .so to a unique temp path before dlopen.
        //
        // Linux's dlopen returns the *cached* handle when the same path is still
        // mapped — even if the file on disk has been replaced. Since old
        // OwnedStoryEntry values hold Arc<Library> keeping the previous .so alive,
        // dlopen on the original path would return the OLD code, not the new build.
        //
        // By copying to a unique path (generation counter), dlopen always maps
        // fresh code. The old temp files are cleaned up when their Library drops.
        let generation = self.load_generation;
        self.load_generation += 1;

        let temp_path = self.so_path.with_extension(format!("so.hot.{generation}"));
        fs::copy(&self.so_path, &temp_path).map_err(LoadSoError::Copy)?;

        // SAFETY: The .so is compiled from the same workspace by the same rustc.
        // Function signatures and type layouts match. We use Rust calling convention
        // (not extern "C") which is correct since both sides share the same ABI.
        let library =
            unsafe { libloading::Library::new(&temp_path) }.map_err(LoadSoError::Dlopen)?;

        let library = Arc::new(library);

        // Initialize the cdylib's asset registrars. Thread-locals are
        // per-shared-object, so we pass the binary's registrar function
        // pointers which call back through RENDERER_PTR.
        unsafe {
            type InitFn = fn(
                fn(&[u8]) -> Option<bmc_wasm_sdk::IconId>,
                fn(&[u8]) -> Option<bmc_wasm_sdk::BitmapId>,
                fn(&[u8]) -> Option<bmc_wasm_sdk::MeshId>,
                fn(&[u8]) -> Option<bmc_wasm_sdk::BitmapId>,
            );
            let init: libloading::Symbol<'_, InitFn> = library
                .get(b"__init_registrars")
                .map_err(LoadSoError::SymbolLookup)?;
            init(
                crate::app::registrar_icon,
                crate::app::registrar_bitmap,
                crate::app::registrar_mesh,
                crate::app::registrar_bitmap_nearest,
            );
        }

        let manifest: StoryManifest = unsafe {
            let func: libloading::Symbol<'_, fn() -> StoryManifest> = library
                .get(b"__story_entries")
                .map_err(LoadSoError::SymbolLookup)?;
            func()
        };

        // Clone strings into owned types BEFORE we could ever drop the library.
        let entries: Vec<OwnedStoryEntry> = manifest
            .entries
            .iter()
            .map(|e| OwnedStoryEntry::from_dynamic(e, &library))
            .collect();

        let groups: Vec<OwnedStoryGroupMeta> = manifest
            .groups
            .iter()
            .map(OwnedStoryGroupMeta::from_static)
            .collect();

        tracing::info!(
            stories = entries.len(),
            groups = groups.len(),
            generation,
            "hot-reload: loaded .so"
        );

        // Replace the library reference. Old entries holding Arc<Library> keep
        // the old .so alive until they are dropped.
        self.library = Some(library);

        // Clean up the previous generation's temp file (best effort).
        //
        // Older `OwnedStoryEntry` values may still hold `Arc<Library>` to this file.
        // On Linux + macOS (the platforms storybook targets), `unlink` removes only
        // the directory entry — the inode and any active `mmap` region stay live
        // until the last mapping is released, so removing while mapped is safe.
        // Windows wouldn't allow it, but storybook isn't supported there.
        if generation > 0 {
            let prev_path = self
                .so_path
                .with_extension(format!("so.hot.{}", generation - 1));
            let _ = fs::remove_file(prev_path);
        }

        Ok((entries, groups))
    }
}

impl Drop for HotReloader {
    fn drop(&mut self) {
        // Kill any in-flight cargo build so it doesn't linger after exit.
        if let Some(mut child) = self.build_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        // Drop the library and entries first so dlclose happens before we delete.
        self.library = None;

        // Remove all temp .so files from this session.
        cleanup_temp_so_files(&self.so_path);
    }
}

/// Remove stale `*.so.hot.*` temp files left behind by previous runs (e.g. after crash).
pub fn cleanup_temp_so_files(so_path: &Path) {
    let Some(parent) = so_path.parent() else {
        return;
    };
    let Some(stem) = so_path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let prefix = format!("{stem}.hot.");
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str()
            && name.starts_with(&prefix)
        {
            tracing::debug!(file = name, "hot-reload: cleaning up stale temp .so");
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Compute the default .so path from the workspace layout.
///
/// Uses `CARGO_MANIFEST_DIR` (set at build time for bmc-storybook) to find the
/// workspace root, then appends `target/debug/libbmc_storybook_stories.so`.
#[must_use]
pub fn default_so_path() -> PathBuf {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: bmc-storybook must be inside workspace");
    workspace_root.join("target/debug/libbmc_storybook_stories.so")
}

/// Compute the workspace root directory.
#[must_use]
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: bmc-storybook must be inside workspace")
        .to_owned()
}

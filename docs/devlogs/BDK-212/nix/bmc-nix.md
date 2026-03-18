# `bmc-nix` Crate Architecture

## Placement

New crate at `bmc-nix/`, added to workspace members. Follows the same patterns as `bmc-upgrade` and `bmc-scheduler` — a library crate exposing functions, no binary of its own (the daemon is a task in the main app for now).

## Module Structure

```
bmc-nix/
├── Cargo.toml
└── src/
    ├── lib.rs                  # Public API re-exports
    ├── types.rs                # Shared data types (Index, Manifest, Package, ServerConfig, GcConfig, etc.)
    ├── index.rs                # Remote index fetching, parsing, merging across servers, Index type
    ├── store.rs                # Nix store operations (init, nix copy, nix-collect-garbage, hash verification)
    ├── manifest.rs             # Profile manifest read/write/diff (what's installed vs what's available)
    ├── profile.rs              # Profile building: symlink trees, generation management, activation handoff
    ├── hooks.rs                # Hook discovery, ordering, execution during profile build
    ├── activation.rs           # Activation script ordering (lexicographic), execution
    ├── upgrade.rs              # Orchestrates the upgrade flow: index check → copy → build → activate
    └── gc.rs                   # Garbage collection logic (generation cleanup, nix-collect-garbage)
```

## Key Design Decisions

### 1. Pure functions + minimal state

Each module exposes async functions taking explicit parameters (paths, configs, HTTP client). No global state or singletons. This makes it trivial to later wrap in a gRPC service, CLI tool, or call from tests.

```rust
// Example: index.rs
pub async fn fetch_indexes(
    client: &reqwest::Client,
    servers: &[ServerEntry],
) -> Result<MergedIndex, FetchIndexesError> { ... }

// Example: index.rs
pub async fn fetch_and_merge_indexes(
    client: &reqwest::Client,
    servers: &[ServerEntry],
) -> Result<MergedIndex, FetchIndexesError> { ... }

// Example: profile.rs
pub async fn build_profile(
    profile_dir: &Path,
    generation: u32,
    packages: &[ResolvedPackage]
) -> Result<ProfileGeneration, BuildProfileError> { ... }

// Example: store.rs
pub async fn copy_store_paths(
    packages: &[ResolvedPackage],
) -> Result<(), CopyStorePathsError> { ... }
```

### 2. Types module — direct from the concept doc

All the JSON structures from the concept doc become serde types:

```rust
// types.rs (sketch)

#[derive(Debug, Clone, Serialize, Deserialize, strum::EnumString, strum::Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum PinStrategy {
    None,
    Major,
    Minor,
    Patch,
}

/// Remote package index (miniminer-index.json)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageIndex {
    pub version: u32,
    pub provenance: Option<Provenance>,
    pub indexes: Vec<Url>,
    pub caches: Vec<CacheEntry>,
    pub packages: Vec<PackageEntry>,
}

/// A single cache entry from the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub name: String,
    pub cache_url: String,
    pub cache_key: String,
}

/// A package entry as it appears in the remote index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    pub name: String,
    pub version: String,
    pub cache: Option<String>,      // cache name, default cache used if absent
    pub store_path: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub upgrade_strategy: Option<UpgradeStrategy>,
    pub install_strategy: Option<InstallStrategy>,
    #[serde(skip)]
    pub server_id: String,          // populated during index merging, not in JSON
}

/// Result of merging indexes from all servers.
/// Maintains both a flat list and a by-name lookup map for efficient
/// searching. Built from the flat Vec during index merging.
#[derive(Debug, Clone)]
pub struct MergedIndex {
    pub caches: Vec<CacheEntry>,
    /// All entries in insertion order
    pub packages: Vec<PackageEntry>,
    /// Lookup by package name → indices into `packages`
    pub by_name: BTreeMap<String, Vec<usize>>,
}

/// A fully resolved package ready for installation.
/// Produced by resolving a PackageEntry against caches and server info.
#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    pub store_path: String,
    pub cache_url: String,          // resolved from CacheEntry
    pub category: Option<String>,
    pub description: Option<String>,
    pub upgrade_strategy: Option<UpgradeStrategy>,
    pub install_strategy: Option<InstallStrategy>,
    pub installed_by: InstalledBy,
    pub installed_from: String,     // server id this package was resolved from
    pub pinned: PinStrategy,
}

/// Factory initialization index (factory.json)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryIndex {
    pub version: u32,
    pub tarballs: Vec<FactoryTarball>,
}

/// Profile manifest (stored in each generation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub packages: BTreeMap<String, ManifestPackage>,
}

/// Per-package manifest entry (extends index entry with installed_by, pinned)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestPackage {
    pub version: String,
    pub cache: String,
    pub store_path: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub upgrade_strategy: Option<UpgradeStrategy>,
    pub install_strategy: Option<InstallStrategy>,
    pub installed_by: InstalledBy,
    pub installed_from: String,     // server id from servers.json, e.g. "braiins_server"
    pub pinned: PinStrategy,        // "none", "major", "minor", "patch"
}

/// What initiated the installation of a package
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstalledBy {
    /// Installed by the system (core packages, auto-installed)
    System,
    /// Installed by explicit user action
    User,
}

/// Upgrade strategy hints for UI and orchestration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpgradeStrategy {
    Reboot,
}

/// Install strategy hints for UI and orchestration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallStrategy {
    Reboot,
}

/// Server registry (/etc/nix-upgrade/servers.json)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServersConfig {
    pub factory: FactoryServerEntry,
    pub servers: Vec<ServerEntry>,
}

/// GC configuration (/etc/nix-upgrade/gc.json)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcConfig {
    pub keep_generations: u32,
    pub keep_days: u32,
    pub min_free_space: String,
    pub protected_generations: Vec<u32>,
}
```

### 3. Error handling — action-specific errors (no single error type)

Each public action returns its own error enum. This keeps error
surfaces precise and makes it possible to exhaustively handle all
failures for a given action.

```rust
// index.rs
#[derive(Debug, thiserror::Error)]
pub enum FetchIndexesError {
    #[error("failed to fetch index from {url}: {source}")]
    Fetch { url: String, source: reqwest::Error },
    #[error("invalid index JSON from {url}: {source}")]
    InvalidJson { url: String, source: serde_json::Error },
    #[error("unsupported index version {version} from {url}")]
    UnsupportedVersion { url: String, version: u32 },
}

// store.rs
#[derive(Debug, thiserror::Error)]
pub enum CopyStorePathsError {
    #[error("store path not available: {0}")]
    StorePathUnavailable(String),
    #[error("nix command failed: {command}")]
    NixCommand { command: String, source: std::io::Error },
    #[error("invalid signature for {store_path}")]
    SignatureInvalid { store_path: String },
    #[error("nar hash mismatch for {store_path}")]
    NarHashMismatch { store_path: String },
}

// profile.rs
#[derive(Debug, thiserror::Error)]
pub enum BuildProfileError {
    #[error("profile build failed: {0}")]
    Build(String),
    #[error(transparent)]
    Hooks(#[from] RunHooksError),
    #[error("manifest error: {0}")]
    Manifest(ManifestError),
    #[error("symlink conflict: {path}")]
    SymlinkConflict { path: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest read failed: {0}")]
    Read(#[source] std::io::Error),
    #[error("manifest write failed: {0}")]
    Write(#[source] std::io::Error),
    #[error("manifest parse failed: {0}")]
    Parse(#[source] serde_json::Error),
}

// activation.rs
#[derive(Debug, thiserror::Error)]
pub enum RunActivationError {
    #[error("activation failed: {script}")]
    ScriptFailed { script: String, source: std::io::Error },
    #[error("activation script ordering failed: {0}")]
    OrderResolution(String),
}

// profile.rs
#[derive(Debug, thiserror::Error)]
pub enum ActivateProfileError {
    #[error("activation failed: {0}")]
    Activation(std::io::Error),
}

// hooks.rs
#[derive(Debug, thiserror::Error)]
pub enum RunHooksError {
    #[error("hook execution failed: {hook}")]
    HookFailed { hook: String, source: std::io::Error },
}

// manifest.rs
#[derive(Debug, thiserror::Error)]
pub enum ReadManifestError {
    #[error("manifest read failed: {0}")]
    Read(#[source] std::io::Error),
    #[error("manifest parse failed: {0}")]
    Parse(#[source] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ComputeUpgradePlanError {
    #[error("upgrade plan error: {0}")]
    Plan(String),
}

/// Output of computing an upgrade plan.
pub struct UpgradePlan {
    /// Resolved packages to apply (includes unchanged packages).
    pub packages: Vec<ResolvedPackage>,
    /// Packages missing from indexes; kept at current version.
    pub stale: Vec<ManifestPackage>,
    /// Packages newly added in the target profile (name + version).
    pub added: Vec<PackageVersion>,
    /// Packages removed in the target profile (name + version).
    pub removed: Vec<PackageVersion>,
    /// Packages that change version (name + from/to).
    pub changed: Vec<PackageChange>,
}

/// A package change between current and target profiles.
pub struct PackageChange {
    pub name: String,
    pub from_version: String,
    pub to_version: String,
}

/// Name + version tuple for upgrade plan diff output.
pub struct PackageVersion {
    pub name: String,
    pub version: String,
}

/// Given the current profile manifest and a merged index, produce
/// a list of resolved packages with versions bumped to what the
/// index offers. Respects the `pinned` strategy of each package.
/// Packages in the manifest that are missing from the index are
/// kept at current version and reported as stale.
/// Additional resolved packages can be added, and packages from the
/// current manifest can be removed.
pub fn compute_upgrade_plan(
    current: &Manifest,
    merged: Option<&MergedIndex>,
    add_packages: &[ResolvedPackage],
    remove_packages: &[ManifestPackage],
) -> Result<UpgradePlan, ComputeUpgradePlanError> { ... }

// index.rs
#[derive(Debug, thiserror::Error)]
pub enum ResolvePackageError {
    #[error("package {0} not found in any index")]
    PackageNotFound(String),
    #[error("no version matching {constraint} for package {package}")]
    VersionNotFound { package: String, constraint: String },
    #[error("cache {cache} referenced by package {package} not found in index from server {server}")]
    CacheNotFound { package: String, cache: String, server: String },
}

/// Resolve a new package by name and optional version constraint.
/// If `version` is None, picks the latest version available.
/// If `version` is Some, picks the best match (e.g. "1.2" matches "1.2.3").
/// The caller provides installed_by since this is a new installation.
/// If a package is not found in the requested server or is a new package,
/// it is resolved by index priority (lower = higher priority).
pub fn resolve_new_package(
    merged: &MergedIndex,
    name: &str,
    version: Option<&str>,
    installed_by: InstalledBy,
) -> Result<ResolvedPackage, ResolvePackageError> { ... }

/// Resolve an already-installed package to its upgraded version.
/// Uses the manifest entry to determine the source server, pinning
/// strategy, and installed_by. Picks the best available version
/// from the same server (by installed_from), respecting the pin. If
/// the package is missing from the original server, resolves by index
/// priority (lower = higher priority).
pub fn resolve_installed_package(
    merged: &MergedIndex,
    name: &str,
    current: &ManifestPackage,
) -> Result<ResolvedPackage, ResolvePackageError> { ... }

// store.rs
#[derive(Debug, thiserror::Error)]
pub enum InitStoreError {
    #[error("failed to fetch factory index from {url}: {source}")]
    FactoryIndexFetch { url: String, source: reqwest::Error },
    #[error("invalid factory index JSON from {url}: {source}")]
    FactoryIndexParse { url: String, source: serde_json::Error },
    #[error("No factory /nix/store for current BOS version.")]
    MissingBosVersion,
    #[error("store init failed: {0}")]
    Init(std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum CollectGarbageError {
    #[error("nix-collect-garbage failed: {0}")]
    NixCommand(std::io::Error),
}

// upgrade.rs
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    #[error(transparent)]
    FetchIndexes(#[from] FetchIndexesError),
    #[error(transparent)]
    ReadManifest(#[from] ReadManifestError),
    #[error(transparent)]
    ComputeUpgradePlan(#[from] ComputeUpgradePlanError),
    #[error(transparent)]
    ResolvePackage(#[from] ResolvePackageError),
    #[error(transparent)]
    Install(#[from] InstallError),
}

// upgrade.rs
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error(transparent)]
    ReadManifest(#[from] ReadManifestError),
    #[error(transparent)]
    CopyStorePaths(#[from] CopyStorePathsError),
    #[error(transparent)]
    BuildProfile(#[from] BuildProfileError),
    #[error(transparent)]
    RunActivation(#[from] RunActivationError),
    #[error(transparent)]
    ActivateProfile(#[from] ActivateProfileError),
    #[error(transparent)]
    CleanupGenerations(#[from] CleanupGenerationsError),
}

// gc.rs
#[derive(Debug, thiserror::Error)]
pub enum CleanupGenerationsError {
    #[error("gc configuration error: {0}")]
    Config(String),
    #[error("generation cleanup failed: {0}")]
    Cleanup(std::io::Error),
}

```

### 4. Nix store operations


```rust
// store.rs
pub async fn nix_copy(
    packages: &[ResolvedPackage],
) -> Result<(), CopyStorePathsError> { ... }

pub async fn init_store(
    client: &reqwest::Client,
    factory_server: &FactoryServerEntry,
    bos_version: &str,
) -> Result<(), InitStoreError> { ... }

```

`init_store` is responsible for fetching the factory index from
`factory_server.index_url`, selecting the matching tarball for the
current BOS version, and initializing the Nix store from it. It also
uses the tarball's `profile_path` to locate and activate the initial profile after
extraction.

### 5. Garbage collection — generations + nix GC

```rust
// gc.rs
pub async fn cleanup_generations(
    profile_dir: &Path,
    gc_config: &GcConfig,
) -> Result<(), CleanupGenerationsError> { ... }

pub async fn collect_garbage() -> Result<(), CollectGarbageError> { ... }
```

### 6. Profile building — symlink tree construction

This is the most complex piece. Pure Rust, no Nix dependency:

```rust
// profile.rs
pub struct ProfileGeneration {
    pub number: u32,
    pub path: PathBuf,
    pub manifest: Manifest,
}

/// Compute the next generation number based on existing generations.
pub async fn next_generation_number(
    profile_dir: &Path,
) -> Result<u32, std::io::Error> { ... }

/// Build a unified symlink tree from all package store paths.
pub async fn build_symlink_tree(
    tmp_path: &Path,
    packages: &[ResolvedPackage],
) -> Result<(), BuildProfileError> { ... }

/// Build a manifest from resolved packages.
pub fn build_manifest(
    packages: &[ResolvedPackage],
) -> Manifest { ... }

/// Write a manifest into the profile directory.
pub async fn write_manifest(
    profile_path: &Path,
    manifest: &Manifest,
) -> Result<(), ManifestError> { ... }

/// Build a new generation from resolved packages
pub async fn build_profile(
    profile_dir: &Path,          // e.g. /nix/var/nix/gcroots/profiles/bmc
    generation: u32,
    packages: &[ResolvedPackage],
    hooks_dir_name: &str,
    hooks_override_path: Option<&Path>,  // native hooks for cross-compilation bootstrap
) -> Result<ProfileGeneration, BuildProfileError> {
    let gen_number = generation;
    let gen_name = format!("{gen_number}-link");
    let gen_path = profile_dir.join(&gen_name);
    let tmp_path = profile_dir.join(format!("{gen_name}.tmp"));

    // 1. Walk store paths, create unified symlink tree
    build_symlink_tree(&tmp_path, packages).await?;

    // 2. Run hooks (file mergers, symlinker, activation script ordering)
    //    When hooks_override_path is set, hooks are executed from that path
    //    instead of from the profile. This is needed for cross-compilation
    //    bootstrap: the profile contains ARM hooks but we need to run native
    //    (x86_64) hooks during init tarball builds.
    hooks::run_hooks(&tmp_path, hooks_dir_name, hooks_override_path).await?;

    // 3. Write manifest
    let manifest = build_manifest(packages);
    write_manifest(&tmp_path, &manifest).await?;

    // 4. Promote tmp to generation path
    tokio::fs::rename(&tmp_path, &gen_path).await.map_err(BuildProfileError::Build)?;

    Ok(ProfileGeneration { number: gen_number, path: gen_path, manifest })
}

/// Activation runner
/// Performs validation/checks first, then atomically switches the
/// current profile symlink as part of the activation scripts.
pub async fn activate_profile(
    profile_dir: &Path,
    generation: u32,
) -> Result<(), ActivateProfileError> {
    let gen_name = format!("{generation}-link");
    let gen_path = profile_dir.join(&gen_name);
    let scripts_dir = gen_path.join("core/activation/scripts");
    let entries = hooks::sorted_dir_entries(&scripts_dir)
        .await
        .map_err(ActivateProfileError::Activation)?;
    for entry in entries {
        let status = tokio::process::Command::new(entry.path())
            .env("PROFILE_NEW_GENERATION", &gen_path)
            .env(
                "PROFILE_OLD_GENERATION",
                current_generation_path(profile_dir).await.as_deref().unwrap_or(Path::new("")),
            )
            .status()
            .await
            .map_err(ActivateProfileError::Activation)?;
        if !status.success() {
            return Err(ActivateProfileError::Activation(std::io::Error::new(
                std::io::ErrorKind::Other,
                "activation script failed",
            )));
        }
    }
    Ok(())
}

/// Resolve the current generation path (symlink target).
pub async fn current_generation_path(
    profile_dir: &Path,
) -> Result<Option<PathBuf>, std::io::Error> { ... }
```

### 7. Hooks — lexicographic execution with env vars

```rust
// hooks.rs
pub async fn run_hooks(
    new_gen_path: &Path,
    hooks_dir_name: &str,
    hooks_override_path: Option<&Path>,  // native hooks for cross-compilation bootstrap
) -> Result<(), RunHooksError> {
    // When override path is set, discover hooks from there instead of
    // from the profile. This handles the case where the profile contains
    // ARM hooks but we're running on x86_64 during init tarball builds.
    let hooks_dir = match hooks_override_path {
        Some(override_path) => override_path.to_path_buf(),
        None => new_gen_path.join(hooks_dir_name),
    };
    if !hooks_dir.exists() { return Ok(()); }

    let mut entries = sorted_dir_entries(&hooks_dir).await?;
    for entry in entries {
        let mut cmd = tokio::process::Command::new(entry.path());
        cmd.env("PROFILE_NEW_GENERATION", new_gen_path);
        let status = cmd.status().await?;
        if !status.success() {
            return Err(RunHooksError::HookFailed { hook: entry.file_name()... });
        }
    }
    Ok(())
}

/// Read directory entries in a stable, lexicographic order.
pub async fn sorted_dir_entries(
    dir: &Path,
) -> Result<Vec<std::fs::DirEntry>, std::io::Error> { ... }
```

### 8. Apply orchestration

`apply_profile_change` is the low-level building block — it takes already-resolved
packages (`&[ResolvedPackage]`) and extends the current profile with
them. The caller is responsible for index fetching, package resolution,
and running checker packages for compatibility. Activation is optional
via a flag.
`apply_profile_change` returns
an `InstallResult` that includes the strategies observed in the run
(e.g. reboot-required), so the UI can act accordingly.

```rust
// upgrade.rs

/// Summary of strategies present in a given install/upgrade run.
pub struct StrategySummary {
    pub upgrade: Vec<UpgradeStrategy>,
    pub install: Vec<InstallStrategy>,
}

/// Result of an install/upgrade run. Used by UI to decide on reboots.
pub struct InstallResult {
    pub generation: ProfileGeneration,
    pub strategies: StrategySummary,
}

/// Merge the current manifest with new packages.
/// Replaces existing entries by name unless pinned.
pub fn merge_installed_with_new(
    current: &Manifest,
    packages: &[ResolvedPackage],
) -> Result<Vec<ResolvedPackage>, ManifestError> { ... }

/// Apply already-resolved packages into the current profile.
/// The caller is responsible for fetching indexes, resolving
/// package names, and running checker packages beforehand.
/// Existing packages are replaced by name unless pinned.
pub async fn apply_profile_change(
    current: &ProfileGeneration,
    profile_dir: &Path,
    gc_config: &GcConfig,
    plan: &UpgradePlan,
    activate: bool,
) -> Result<InstallResult, InstallError> {
    // 1. Read current manifest — existing packages carry over unchanged
    let all_packages = manifest::merge_installed_with_new(
        &current.manifest,
        &plan.packages,
    )?;

    // 2. Copy store paths (only new ones, existing are already in store)
    store::copy_store_paths(&plan.packages).await?;

    // 3. Build new profile generation (includes existing + new packages)
    let gen_number = profile::next_generation_number(profile_dir).await?;
    let generation = profile::build_profile(
        profile_dir, gen_number, &all_packages, "hooks",
    ).await?;

    // 4. Run activation scripts (optional)
    if activate {
        profile::activate_profile(profile_dir, generation.number).await?;
    }

    Ok(InstallResult {
        generation,
        strategies: StrategySummary::from_packages(&all_packages),
    })
}
```

## Dependencies

```toml
[dependencies]
anyhow.workspace = true
reqwest = { workspace = true, features = ["json", "rustls-tls"] }
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
thiserror.workspace = true
tokio = { workspace = true, features = ["full"] }
tracing.workspace = true
url.workspace = true
semver.workspace = true
walkdir.workspace = true
```

## Hooks as Separate Binaries

For the common hooks (file-merger, file-symlinker, activation resolver for ordering):

Keep them in `bmc-nix` as `[[bin]]` targets within the same crate. They share the types from `bmc-nix` directly.

```toml
# bmc-nix/Cargo.toml
[[bin]]
name = "bmc-hook-merge-files"
path = "src/bin/hook_merge_files.rs"

[[bin]]
name = "bmc-hook-file-symlinks"
path = "src/bin/hook_file_symlinks.rs"

[[bin]]
name = "bmc-hook-activation-resolver"
path = "src/bin/hook_activation_resolver.rs"
```

## Integration with the Main App

For now, the daemon is just a task spawned in `bmc/src/startup.rs` or `bmc-openwrt/src/main.rs`. The `bmc-nix` library provides all the building blocks as functions. The calling code decides when to invoke them (on boot for init check, on user request for upgrade, on timer for GC).

## Summary

| Concern | Where |
|---|---|
| Data types & JSON schemas | `bmc-nix/src/types.rs` |
| Remote index fetching/merging | `bmc-nix/src/index.rs` |
| Nix store init/copy/gc | `bmc-nix/src/store.rs` |
| Manifest read/write/diff | `bmc-nix/src/manifest.rs` |
| Profile symlink tree building | `bmc-nix/src/profile.rs` |
| Hook discovery & execution | `bmc-nix/src/hooks.rs` |
| Activation script ordering | `bmc-nix/src/activation.rs` |
| Upgrade + install orchestration | `bmc-nix/src/upgrade.rs` |
| GC policy | `bmc-nix/src/gc.rs` |
| Hook binaries | `bmc-nix/src/bin/hook_*.rs` |

The key principle: **all operations are plain async functions with explicit parameters**. No framework, no trait hierarchy, no daemon loop built in. The caller (main app task, future CLI, future gRPC service) composes them as needed.

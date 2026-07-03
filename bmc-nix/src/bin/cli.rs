// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use bmc_nix::manifest;
use bmc_nix::types::{
    BaseSelector, FetchedIndex, GcConfig, InstallResult, Manifest, MergedIndex, PackageChange,
    PackageVersion, ServerEntry, ServersConfig,
};
use bmc_nix::upgrade::ActivationMode;
use clap::{Parser, Subcommand};

#[path = "cli/progress.rs"]
mod progress;

/// Per-request timeout for fetching upgrade indexes over HTTP.
const INDEX_FETCH_TIMEOUT_SECS: u64 = 30;

/// Print a human-readable diff of an `InstallResult` on stderr.
///
/// Format (matches spec §4):
///
/// ```text
/// Profile change: +N added, -M removed, K changed
///   + pkg version
///   - pkg version
///   ~ pkg: from -> to
/// ```
///
/// When only a store path changed (same version), the change line is
/// rendered as `~ pkg: version (store path changed)`.
///
/// Prints `Profile unchanged.` when the diff is empty.
///
/// A garbage-collection failure is reported as a warning: the profile
/// change itself already succeeded, only the post-activation cleanup did
/// not.
fn print_profile_diff(result: &InstallResult) {
    if let Err(err) = &result.gc {
        eprintln!("Warning: profile updated but garbage collection failed: {err}");
    }

    if result.added.is_empty() && result.removed.is_empty() && result.changed.is_empty() {
        eprintln!("Profile unchanged.");
        return;
    }

    eprintln!(
        "Profile change: +{} added, -{} removed, {} changed",
        result.added.len(),
        result.removed.len(),
        result.changed.len(),
    );

    let mut added: Vec<&PackageVersion> = result.added.iter().collect();
    added.sort_by(|a, b| a.name.cmp(&b.name));
    for pv in added {
        eprintln!("  + {} {}", pv.name, pv.version);
    }

    let mut removed: Vec<&PackageVersion> = result.removed.iter().collect();
    removed.sort_by(|a, b| a.name.cmp(&b.name));
    for pv in removed {
        eprintln!("  - {} {}", pv.name, pv.version);
    }

    let mut changed: Vec<&PackageChange> = result.changed.iter().collect();
    changed.sort_by(|a, b| a.name.cmp(&b.name));
    for ch in changed {
        if ch.from_version == ch.to_version {
            eprintln!("  ~ {}: {} (store path changed)", ch.name, ch.from_version);
        } else {
            eprintln!("  ~ {}: {} -> {}", ch.name, ch.from_version, ch.to_version);
        }
    }
}

/// Render the stale-package warning for an `upgrade`, or `None` when no
/// installed package fell out of the merged indexes.
///
/// Stale packages are kept at their installed version; the warning tells
/// the operator they are pinned to whatever the device already has.
fn format_stale_warning(stale: &[PackageVersion]) -> Option<String> {
    use std::fmt::Write as _;
    if stale.is_empty() {
        return None;
    }
    let mut sorted: Vec<&PackageVersion> = stale.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = format!(
        "Warning: {} package(s) kept at installed version (absent from the indexes or no in-pin upgrade available):",
        sorted.len(),
    );
    for pv in sorted {
        let _ = write!(out, "\n  ! {} {}", pv.name, pv.version);
    }
    Some(out)
}

/// Output format for realization progress (on stderr).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum LogFormat {
    /// Throttled, human-readable lines.
    #[default]
    Human,
    /// Line-delimited JSON, one object per event, each prefixed `@bmc `.
    InternalJson,
}

/// Top-level CLI for bmc-nix profile management.
#[derive(Debug, Parser)]
#[command(name = "bmc-nix-cli")]
struct Cli {
    /// Progress output format on stderr (stdout is unchanged).
    #[arg(long, global = true, default_value_t = LogFormat::Human, value_enum)]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Commands,
}

/// Flags shared by the three profile-mutating subcommands that operate
/// on an existing profile (`AddPackages`, `RemovePackages`,
/// `ResetProfile`).
///
/// `BuildProfile` does NOT flatten this struct — its `--profile-dir`
/// is required (no default), and flattening would either change the
/// CLI surface or require splitting the struct.
#[derive(clap::Args, Debug)]
struct ProfileCommonArgs {
    /// Directory for the profile generations.
    #[arg(long, default_value = "/nix/var/nix/gcroots/profiles/bmc")]
    profile_dir: PathBuf,

    /// Name of the hooks directory inside the profile (default: "hooks").
    #[arg(long, default_value = "hooks")]
    hooks_dir: String,

    /// Override path for hook executables (for cross-compilation bootstrap).
    #[arg(long)]
    hooks_override_path: Option<PathBuf>,

    /// Build the generation but do not create/update the `current`
    /// symlink. Activation is the default.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    no_activate: bool,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
enum Commands {
    // `BuildProfile` keeps its args inline: its `--profile-dir` is
    // required (no default), so it cannot share `ProfileCommonArgs`
    // without either changing the CLI surface (accepting the default)
    // or splitting the struct. Neither is worth the trade for a
    // single outlier.
    /// Build a profile from an index JSON file
    BuildProfile {
        /// Path to nix-package-index.v1.json
        #[arg(long)]
        index: PathBuf,

        /// Directory for the profile generations
        #[arg(long)]
        profile_dir: PathBuf,

        /// Name of the hooks directory inside the profile (default: "hooks")
        #[arg(long, default_value = "hooks")]
        hooks_dir: String,

        /// Override path for hook executables (for cross-compilation bootstrap)
        #[arg(long)]
        hooks_override_path: Option<PathBuf>,

        /// Build the generation but do not create/update the `current`
        /// symlink. Activation is the default.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        no_activate: bool,
    },

    /// Add one or more local packages to a profile
    AddPackages {
        /// Package name (repeatable, positionally paired with --version and --store-path)
        #[arg(long)]
        name: Vec<String>,

        /// Package version (repeatable, positionally paired with --name and --store-path)
        #[arg(long)]
        version: Vec<String>,

        /// Nix store path (repeatable, positionally paired with --name and --version)
        #[arg(long)]
        store_path: Vec<String>,

        /// Base generation to diff against: `current` (default),
        /// `latest`, or a positive integer generation number.
        #[arg(long, default_value = "current")]
        base: BaseSelector,

        #[command(flatten)]
        common: ProfileCommonArgs,
    },

    /// Remove packages from a profile
    RemovePackages {
        /// Package names to remove
        #[arg(long = "name", required = true)]
        names: Vec<String>,

        /// Base generation to diff against: `current` (default),
        /// `latest`, or a positive integer generation number.
        #[arg(long, default_value = "current")]
        base: BaseSelector,

        #[command(flatten)]
        common: ProfileCommonArgs,
    },

    /// Reset profile from an index JSON (no manifest merging)
    ResetProfile {
        /// Path to nix-package-index.v1.json
        #[arg(long)]
        index: PathBuf,

        #[command(flatten)]
        common: ProfileCommonArgs,
    },

    /// Upgrade installed packages against the configured indexes
    Upgrade {
        /// Path to the server registry
        /// (default: /etc/nix-upgrade/servers.json).
        #[arg(long)]
        servers_config: Option<PathBuf>,

        /// Ad-hoc index reference (repeatable): an http(s) base URL, or a
        /// `file://` path to an index JSON. Highest precedence; a fetch
        /// failure aborts the run.
        #[arg(long = "index")]
        indexes: Vec<String>,

        /// Resolve exclusively against the --index references; do not read
        /// or consult the server registry.
        #[arg(long, action = clap::ArgAction::SetTrue, requires = "indexes")]
        only_indexes: bool,

        /// Base generation to diff against: `current` (default),
        /// `latest`, or a positive integer generation number.
        #[arg(long, default_value = "current")]
        base: BaseSelector,

        /// Build the new generation but defer activation to the next boot:
        /// write the `next` marker consumed by the boot-time activator
        /// instead of swapping `current`.
        #[arg(
            long,
            action = clap::ArgAction::SetTrue,
            conflicts_with = "no_activate"
        )]
        next_boot: bool,

        #[command(flatten)]
        common: ProfileCommonArgs,
    },

    /// Initialize the Nix store from the configured factory tarball
    Init {
        /// Path to the server registry.
        #[arg(long, default_value = "/etc/nix-upgrade/servers.json")]
        servers_config: PathBuf,

        /// Data partition device to format and mount when needed.
        #[arg(long, default_value = "/dev/mmcblk0p4")]
        data_partition: PathBuf,

        /// Data partition mount point and staged extraction root.
        #[arg(long, default_value = "/mnt/data")]
        data_dir: PathBuf,

        /// File containing the current BOS version.
        #[arg(long, default_value = "/etc/bos_version")]
        bos_version_file: PathBuf,

        /// Directory for the downloaded factory tarball.
        #[arg(long, default_value = "/mnt/data")]
        download_dir: PathBuf,

        /// Replace an existing promoted store at <data-dir>/nix.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        wipe: bool,
    },

    /// Check whether the promoted Nix store exists
    IsInitialized {
        /// Data partition mount point.
        #[arg(long, default_value = "/mnt/data")]
        data_dir: PathBuf,
    },

    /// Prune old profile generations, then collect store garbage
    Gc {
        /// Path to the GC config (default: /etc/nix-upgrade/gc.json).
        /// Falls back to built-in defaults when the file is absent.
        #[arg(long)]
        gc_config: Option<PathBuf>,

        /// Directory for the profile generations.
        #[arg(long, default_value = "/nix/var/nix/gcroots/profiles/bmc")]
        profile_dir: PathBuf,

        /// Override the number of most-recent generations to keep.
        #[arg(long)]
        keep_generations: Option<usize>,

        /// Override the age cutoff: keep generations newer than this many
        /// days.
        #[arg(long)]
        keep_days: Option<usize>,

        /// Override the minimum free space target.
        #[arg(long)]
        min_free_space: Option<String>,

        /// Generation number to protect (repeatable). A non-empty list
        /// replaces the configured `protected_generations`.
        #[arg(long = "protected-generation")]
        protected_generations: Vec<usize>,
    },
}

/// Resolve a `BaseSelector` into the optional `base_manifest` argument
/// for `apply_profile_change`.
///
/// Returns `Ok(None)` for `BaseSelector::Current` — the default path
/// reads the manifest under the profile lock in `apply_profile_change`.
/// For `Latest` / `Generation(N)` the manifest is read now and passed
/// as `Some(_)`.
fn resolve_base(
    profile_dir: &Path,
    selector: &BaseSelector,
) -> Result<Option<Manifest>, manifest::ReadManifestError> {
    match selector {
        BaseSelector::Current => Ok(None),
        BaseSelector::Latest | BaseSelector::Generation(_) => Ok(Some(
            manifest::read_manifest_by_selector(profile_dir, selector)?,
        )),
    }
}

/// Load the server registry from `path`.
///
/// A missing or unparseable file is fatal: `upgrade` consumes the config
/// init already provisioned and never repairs or falls back.
fn load_servers_config(path: &Path) -> anyhow::Result<ServersConfig> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read servers config at {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse servers config at {}", path.display()))
}

/// Load the GC config from `path`, falling back to defaults when absent.
///
/// A present-but-unparseable file is fatal: it signals a provisioning
/// error rather than an unconfigured device.
fn load_gc_config(path: &Path) -> anyhow::Result<GcConfig> {
    if !path.exists() {
        return Ok(GcConfig::default());
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read gc config at {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse gc config at {}", path.display()))
}

/// Apply CLI overrides onto a loaded [`GcConfig`].
///
/// Each `Some` scalar replaces the loaded value; `None` keeps it. A
/// non-empty `protected_generations` replaces the loaded list, an empty
/// one keeps it.
fn apply_gc_overrides(
    config: &mut GcConfig,
    keep_generations: Option<usize>,
    keep_days: Option<usize>,
    min_free_space: Option<String>,
    protected_generations: Vec<usize>,
) {
    if let Some(value) = keep_generations {
        config.keep_generations = value;
    }
    if let Some(value) = keep_days {
        config.keep_days = Some(value);
    }
    if let Some(value) = min_free_space {
        config.min_free_space = value;
    }
    if !protected_generations.is_empty() {
        config.protected_generations = protected_generations;
    }
}

/// Build the fetch set: the configured servers plus one synthetic
/// entry per `--index` reference.
///
/// Custom entries sit at priority 0 (highest precedence), so a custom
/// index wins a version tie against any configured server. Ids are
/// `custom-<n>` by flag order; the `custom` type is a stable marker and
/// is not consulted during resolution.
fn build_fetch_set(
    mut configured: Vec<ServerEntry>,
    custom_indexes: &[String],
) -> Vec<ServerEntry> {
    for (i, reference) in custom_indexes.iter().enumerate() {
        configured.push(ServerEntry {
            id: format!("custom-{i}"),
            server_type: "custom".to_owned(),
            base_url: reference.clone(),
            known_public_key: String::new(),
            priority: 0,
            enabled: true,
        });
    }
    configured
}

/// Fail when the fetch set has no enabled entry to fetch from.
///
/// The shipped default `servers.json` carries an empty `servers` list, so
/// a plain `upgrade` with no `--index` would otherwise fetch nothing,
/// resolve every package as absent and mislabel them all as stale.
fn ensure_fetchable(fetch_set: &[ServerEntry]) -> anyhow::Result<()> {
    if fetch_set.iter().any(|s| s.enabled) {
        return Ok(());
    }
    anyhow::bail!("no upgrade index configured: add a server to servers.json or pass --index <url>")
}

async fn fetch_and_merge_primary_indexes(
    client: &reqwest::Client,
    fetch_set: &[ServerEntry],
) -> Result<MergedIndex, bmc_nix::index::FetchIndexesError> {
    let mut enabled_servers: Vec<&ServerEntry> = fetch_set.iter().filter(|s| s.enabled).collect();
    enabled_servers.sort_by_key(|s| s.priority);

    let mut fetched = Vec::with_capacity(enabled_servers.len());
    for server in enabled_servers {
        let index = bmc_nix::index::fetch_index(client, &server.base_url).await?;
        fetched.push(FetchedIndex {
            server_id: server.id.clone(),
            server_priority: server.priority,
            index,
        });
    }

    Ok(bmc_nix::index::merge_indexes(fetched))
}

fn activation_mode_from_no_activate(no_activate: bool) -> ActivationMode {
    if no_activate {
        ActivationMode::Skip
    } else {
        ActivationMode::Activate
    }
}

async fn cmd_build_profile(
    index: PathBuf,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    no_activate: bool,
) -> anyhow::Result<()> {
    let activate = !no_activate;

    let index_content = std::fs::read_to_string(&index)?;
    let package_index: bmc_nix::types::PackageIndex = serde_json::from_str(&index_content)?;
    let packages = bmc_nix::index::resolve_all_from_index(&package_index);

    std::fs::create_dir_all(&profile_dir)?;

    // Hold lock to prevent TOCTOU race on generation number
    let lock = bmc_nix::profile::lock_profile(&profile_dir).await?;
    let generation_number = bmc_nix::profile::max_generation(&profile_dir)?.unwrap_or(0) + 1;
    let generation = bmc_nix::profile::build_profile(
        &profile_dir,
        generation_number,
        &packages,
        &hooks_dir,
        hooks_override_path.as_deref(),
    )
    .await?;

    if activate {
        bmc_nix::profile::activate_profile(
            &profile_dir,
            generation.number,
            &generation.path,
            Some(&lock),
        )
        .await?;
    }

    println!("{}", generation.path.display());
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatch — all args are required"
)]
async fn cmd_add_packages(
    name: Vec<String>,
    version: Vec<String>,
    store_path: Vec<String>,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    base: BaseSelector,
    no_activate: bool,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        name.len() == version.len() && name.len() == store_path.len(),
        "--name, --version, and --store-path must each be provided the same number of times \
         (got {}, {}, {} respectively)",
        name.len(),
        version.len(),
        store_path.len(),
    );

    let add_packages: Vec<bmc_nix::types::ResolvedPackage> = name
        .into_iter()
        .zip(version)
        .zip(store_path)
        .map(|((n, v), sp)| bmc_nix::types::ResolvedPackage {
            name: n,
            version: v,
            store_path: sp,
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: bmc_nix::types::InstalledBy::User,
            installed_from: "local".into(),
            pinned: None,
        })
        .collect();

    std::fs::create_dir_all(&profile_dir)?;

    let base_manifest = resolve_base(&profile_dir, &base)?;

    let progress = progress::CliProgress::new(log_format);
    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        base_manifest,
        None,
        &add_packages,
        &[],
        activation_mode_from_no_activate(no_activate),
        None,
        Some(&progress),
        &hooks_dir,
        hooks_override_path.as_deref(),
    )
    .await?;

    print_profile_diff(&result);
    if let Some(generation) = result.generation {
        println!("{}", generation.path.display());
    }
    Ok(())
}

async fn cmd_remove_packages(
    profile_dir: PathBuf,
    names: Vec<String>,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    base: BaseSelector,
    no_activate: bool,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&profile_dir)?;

    let base_manifest = resolve_base(&profile_dir, &base)?;

    let progress = progress::CliProgress::new(log_format);
    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        base_manifest,
        None,
        &[],
        &names,
        activation_mode_from_no_activate(no_activate),
        None,
        Some(&progress),
        &hooks_dir,
        hooks_override_path.as_deref(),
    )
    .await?;

    print_profile_diff(&result);
    if let Some(generation) = result.generation {
        println!("{}", generation.path.display());
    }
    Ok(())
}

async fn cmd_reset_profile(
    index: PathBuf,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    no_activate: bool,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    let index_content = std::fs::read_to_string(&index)?;
    let package_index: bmc_nix::types::PackageIndex = serde_json::from_str(&index_content)?;
    let packages = bmc_nix::index::resolve_all_from_index(&package_index);

    std::fs::create_dir_all(&profile_dir)?;

    let progress = progress::CliProgress::new(log_format);
    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        Some(Manifest::default()),
        None,
        &packages,
        &[],
        activation_mode_from_no_activate(no_activate),
        None,
        Some(&progress),
        &hooks_dir,
        hooks_override_path.as_deref(),
    )
    .await?;

    let generation = result
        .generation
        .expect("BUG: reset-profile always produces a generation");
    println!("{}", generation.path.display());
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatch — all args are required"
)]
async fn cmd_upgrade(
    servers_config: Option<PathBuf>,
    indexes: Vec<String>,
    only_indexes: bool,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    base: BaseSelector,
    no_activate: bool,
    next_boot: bool,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    let fetch_set = if only_indexes {
        build_fetch_set(Vec::new(), &indexes)
    } else {
        let servers_path =
            servers_config.unwrap_or_else(|| PathBuf::from("/etc/nix-upgrade/servers.json"));
        let config = load_servers_config(&servers_path)?;
        build_fetch_set(config.servers, &indexes)
    };
    ensure_fetchable(&fetch_set)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(INDEX_FETCH_TIMEOUT_SECS))
        .build()
        .expect("BUG: reqwest client builder");
    let merged = if only_indexes {
        fetch_and_merge_primary_indexes(&client, &fetch_set).await?
    } else {
        bmc_nix::index::fetch_and_merge_indexes(&client, &fetch_set).await?
    };

    std::fs::create_dir_all(&profile_dir)?;
    let base_manifest = resolve_base(&profile_dir, &base)?;

    let progress = progress::CliProgress::new(log_format);
    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        base_manifest,
        Some(&merged),
        &[],
        &[],
        if next_boot {
            ActivationMode::NextBoot
        } else {
            activation_mode_from_no_activate(no_activate)
        },
        None,
        Some(&progress),
        &hooks_dir,
        hooks_override_path.as_deref(),
    )
    .await?;

    print_profile_diff(&result);
    if let Some(warning) = format_stale_warning(&result.stale) {
        eprintln!("{warning}");
    }
    if let Some(generation) = result.generation {
        println!("{}", generation.path.display());
    }
    Ok(())
}

/// Build a shared HTTP client for first-boot init downloads.
///
/// TLS cert validation is disabled because NTP has not synced on
/// first boot (clock is at epoch → certs appear "not yet valid").
/// Tarball integrity is ensured by signature verification, not TLS.
fn build_init_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .danger_accept_invalid_certs(true)
        .build()
        .expect("BUG: failed to build HTTP client")
}

fn read_bos_version(path: &Path) -> anyhow::Result<String> {
    Ok(std::fs::read_to_string(path)
        .with_context(|| format!("failed to read BOS version from {}", path.display()))?
        .trim()
        .to_owned())
}

fn is_initialized(data_dir: &Path) -> bool {
    data_dir.join("nix").is_dir()
}

async fn cmd_init(
    servers_config: PathBuf,
    data_partition: PathBuf,
    data_dir: PathBuf,
    bos_version_file: PathBuf,
    download_dir: PathBuf,
    wipe: bool,
) -> anyhow::Result<()> {
    // An active /nix means the system is running from the store this
    // wipe would delete out from under it.
    if wipe && bmc_nix::partition::is_path_mounted(Path::new("/nix"))? {
        anyhow::bail!("refusing to wipe the store: /nix is an active mount");
    }

    bmc_nix::partition::prepare_data_partition(
        &bmc_nix::store::TokioCommandRunner,
        &data_partition,
        &data_dir,
    )
    .await?;

    // The firmware COMMAND distinguishes this no-op from a fresh
    // initialization by stdout: keep it empty here; a fresh init
    // prints the promoted profile path below.
    if is_initialized(&data_dir) && !wipe {
        tracing::info!("store already initialized");
        return Ok(());
    }

    let servers = load_servers_config(&servers_config)?;
    let bos_version = read_bos_version(&bos_version_file)?;
    let client = build_init_http_client();
    let result = bmc_nix::store::init_store(
        &client,
        &servers.factory,
        &bos_version,
        &download_dir,
        &data_dir,
        wipe,
        None,
    )
    .await?;

    println!("{}", result.profile_path.display());
    Ok(())
}

async fn cmd_gc(
    gc_config: Option<PathBuf>,
    profile_dir: PathBuf,
    keep_generations: Option<usize>,
    keep_days: Option<usize>,
    min_free_space: Option<String>,
    protected_generations: Vec<usize>,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    let config_path = gc_config.unwrap_or_else(|| PathBuf::from("/etc/nix-upgrade/gc.json"));
    let mut config = load_gc_config(&config_path)?;
    apply_gc_overrides(
        &mut config,
        keep_generations,
        keep_days,
        min_free_space,
        protected_generations,
    );

    // Hold the profile lock across both mutations: the profiles.md invariant
    // requires every profile mutation to take the lock, and it keeps gc from
    // pruning generations or store paths a concurrent upgrade is realizing.
    let _lock = bmc_nix::profile::lock_profile(&profile_dir).await?;
    bmc_nix::gc::cleanup_generations(&profile_dir, &config, &[])?;
    let progress = progress::CliProgress::new(log_format);
    bmc_nix::gc::collect_garbage(&bmc_nix::store::TokioCommandRunner, Some(&progress)).await?;
    Ok(())
}

/// Install a stderr `tracing` subscriber so emitted events (e.g. gc
/// generation removals) are visible. Stdout is reserved for command
/// output, so logs go to stderr. The level defaults to `info` and is
/// overridable via `RUST_LOG`.
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .init();
}

#[expect(
    clippy::too_many_lines,
    reason = "CLI dispatch — one match arm per subcommand"
)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();

    let cli = Cli::parse();
    let log_format = cli.log_format;

    match cli.command {
        Commands::BuildProfile {
            index,
            profile_dir,
            hooks_dir,
            hooks_override_path,
            no_activate,
        } => {
            cmd_build_profile(
                index,
                profile_dir,
                hooks_dir,
                hooks_override_path,
                no_activate,
            )
            .await
        }

        Commands::AddPackages {
            name,
            version,
            store_path,
            base,
            common,
        } => {
            cmd_add_packages(
                name,
                version,
                store_path,
                common.profile_dir,
                common.hooks_dir,
                common.hooks_override_path,
                base,
                common.no_activate,
                log_format,
            )
            .await
        }

        Commands::RemovePackages {
            names,
            base,
            common,
        } => {
            cmd_remove_packages(
                common.profile_dir,
                names,
                common.hooks_dir,
                common.hooks_override_path,
                base,
                common.no_activate,
                log_format,
            )
            .await
        }

        Commands::ResetProfile { index, common } => {
            cmd_reset_profile(
                index,
                common.profile_dir,
                common.hooks_dir,
                common.hooks_override_path,
                common.no_activate,
                log_format,
            )
            .await
        }

        Commands::Upgrade {
            servers_config,
            indexes,
            only_indexes,
            base,
            next_boot,
            common,
        } => {
            cmd_upgrade(
                servers_config,
                indexes,
                only_indexes,
                common.profile_dir,
                common.hooks_dir,
                common.hooks_override_path,
                base,
                common.no_activate,
                next_boot,
                log_format,
            )
            .await
        }

        Commands::Init {
            servers_config,
            data_partition,
            data_dir,
            bos_version_file,
            download_dir,
            wipe,
        } => {
            cmd_init(
                servers_config,
                data_partition,
                data_dir,
                bos_version_file,
                download_dir,
                wipe,
            )
            .await
        }

        Commands::IsInitialized { data_dir } => {
            if is_initialized(&data_dir) {
                Ok(())
            } else {
                std::process::exit(1)
            }
        }

        Commands::Gc {
            gc_config,
            profile_dir,
            keep_generations,
            keep_days,
            min_free_space,
            protected_generations,
        } => {
            cmd_gc(
                gc_config,
                profile_dir,
                keep_generations,
                keep_days,
                min_free_space,
                protected_generations,
                log_format,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn upgrade_accepts_next_boot_flag() {
        let cli = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--index",
            "file:///tmp/index.json",
            "--next-boot",
        ])
        .expect("BUG: parse should succeed");

        let Commands::Upgrade {
            next_boot, common, ..
        } = cli.command
        else {
            panic!("BUG: parsed command must be upgrade");
        };
        assert!(next_boot, "next-boot flag must be recorded");
        assert!(
            !common.no_activate,
            "next-boot must not imply the no-activate flag"
        );
    }

    #[test]
    fn upgrade_rejects_next_boot_with_no_activate() {
        let err = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--index",
            "file:///tmp/index.json",
            "--next-boot",
            "--no-activate",
        ])
        .expect_err("BUG: next-boot conflicts with no-activate");

        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn upgrade_only_indexes_requires_an_index() {
        let err = Cli::try_parse_from(["bmc-nix-cli", "upgrade", "--only-indexes"])
            .expect_err("BUG: --only-indexes requires at least one --index");

        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn upgrade_accepts_only_indexes_with_explicit_index() {
        let cli = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--only-indexes",
            "--index",
            "file:///tmp/index.json",
        ])
        .expect("BUG: --only-indexes with --index should parse");

        let Commands::Upgrade { only_indexes, .. } = cli.command else {
            panic!("BUG: parsed command must be upgrade");
        };
        assert!(only_indexes, "only-indexes flag must be recorded");
    }

    #[test]
    fn init_download_dir_defaults_to_persistent_storage() {
        let cli = Cli::try_parse_from(["bmc-nix-cli", "init"])
            .expect("BUG: init should parse with defaults");
        let Commands::Init { download_dir, .. } = cli.command else {
            panic!("BUG: parsed command must be init");
        };
        // The factory closure is too large for the /tmp tmpfs; it must land
        // on persistent storage.
        assert_eq!(download_dir, PathBuf::from("/mnt/data"));
    }

    #[test]
    fn is_initialized_defaults_to_data_dir() {
        let cli = Cli::try_parse_from(["bmc-nix-cli", "is-initialized"])
            .expect("BUG: is-initialized should parse with defaults");

        let Commands::IsInitialized { data_dir } = cli.command else {
            panic!("BUG: parsed command must be is-initialized");
        };
        assert_eq!(data_dir, PathBuf::from("/mnt/data"));
    }

    #[test]
    fn is_initialized_requires_promoted_store_directory() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        assert!(!is_initialized(tmp.path()));

        std::fs::create_dir_all(tmp.path().join("nix.tmp/nix")).expect("BUG: setup");
        assert!(
            !is_initialized(tmp.path()),
            "staged but unpromoted store must not count as initialized"
        );

        std::fs::write(tmp.path().join("nix"), "").expect("BUG: setup");
        assert!(
            !is_initialized(tmp.path()),
            "a non-directory nix path must not count as initialized"
        );
        std::fs::remove_file(tmp.path().join("nix")).expect("BUG: cleanup");

        std::fs::create_dir(tmp.path().join("nix")).expect("BUG: setup");
        assert!(is_initialized(tmp.path()));
    }

    #[test]
    fn load_servers_config_reads_valid_file() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("servers.json");
        std::fs::write(
            &path,
            r#"{
                "factory": {"id":"braiins","base_url":"https://cache.braiins.com/v1","known_public_key":"k","priority":0,"enabled":true},
                "servers": [
                    {"id":"s1","type":"mirror","base_url":"https://s1.example.com/v1","known_public_key":"k","priority":10,"enabled":true}
                ]
            }"#,
        )
        .expect("BUG: write servers.json");

        let config = load_servers_config(&path).expect("BUG: valid config should load");
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].id, "s1");
        assert!(config.servers[0].enabled);
    }

    #[test]
    fn load_servers_config_missing_file_is_fatal() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("absent.json");
        assert!(
            load_servers_config(&path).is_err(),
            "missing config must be a fatal error"
        );
    }

    #[test]
    fn load_servers_config_unparseable_file_is_fatal() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("servers.json");
        std::fs::write(&path, "this is not json").expect("BUG: write garbage");
        assert!(
            load_servers_config(&path).is_err(),
            "unparseable config must be a fatal error"
        );
    }

    #[test]
    fn load_gc_config_missing_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("absent.json");
        let config = load_gc_config(&path).expect("BUG: missing file must default, not fail");
        let default = GcConfig::default();
        assert_eq!(config.keep_generations, default.keep_generations);
        assert_eq!(config.keep_days, default.keep_days);
        assert_eq!(config.min_free_space, default.min_free_space);
        assert_eq!(config.protected_generations, default.protected_generations);
    }

    #[test]
    fn load_gc_config_reads_valid_file() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("gc.json");
        std::fs::write(
            &path,
            r#"{"keep_generations":7,"keep_days":14,"min_free_space":"1G","protected_generations":[2,5]}"#,
        )
        .expect("BUG: write gc.json");

        let config = load_gc_config(&path).expect("BUG: valid config should load");
        assert_eq!(config.keep_generations, 7);
        assert_eq!(config.keep_days, Some(14));
        assert_eq!(config.min_free_space, "1G");
        assert_eq!(config.protected_generations, vec![2, 5]);
    }

    #[test]
    fn load_gc_config_partial_file_fills_missing_fields_from_defaults() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("gc.json");
        std::fs::write(&path, r#"{"keep_generations":9}"#).expect("BUG: write gc.json");

        let config = load_gc_config(&path).expect("BUG: partial config should load");
        let default = GcConfig::default();
        assert_eq!(config.keep_generations, 9);
        assert_eq!(config.min_free_space, default.min_free_space);
        assert_eq!(config.protected_generations, default.protected_generations);
    }

    #[test]
    fn load_gc_config_unparseable_present_file_is_fatal() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let path = dir.path().join("gc.json");
        std::fs::write(&path, "this is not json").expect("BUG: write garbage");
        assert!(
            load_gc_config(&path).is_err(),
            "a present-but-unparseable config must be a fatal error"
        );
    }

    #[test]
    fn apply_gc_overrides_replaces_only_provided_fields() {
        let mut config = GcConfig {
            keep_generations: 3,
            keep_days: None,
            min_free_space: "0".to_owned(),
            protected_generations: vec![1],
        };
        apply_gc_overrides(
            &mut config,
            Some(8),
            Some(30),
            Some("2G".to_owned()),
            vec![4, 6],
        );
        assert_eq!(config.keep_generations, 8);
        assert_eq!(config.keep_days, Some(30));
        assert_eq!(config.min_free_space, "2G");
        assert_eq!(config.protected_generations, vec![4, 6]);
    }

    #[test]
    fn apply_gc_overrides_keeps_loaded_values_when_unset() {
        let mut config = GcConfig {
            keep_generations: 3,
            keep_days: Some(10),
            min_free_space: "1G".to_owned(),
            protected_generations: vec![1, 2],
        };
        apply_gc_overrides(&mut config, None, None, None, Vec::new());
        assert_eq!(config.keep_generations, 3);
        assert_eq!(config.keep_days, Some(10));
        assert_eq!(config.min_free_space, "1G");
        assert_eq!(config.protected_generations, vec![1, 2]);
    }

    fn configured_server(id: &str, priority: u32) -> ServerEntry {
        ServerEntry {
            id: id.to_owned(),
            server_type: "mirror".to_owned(),
            base_url: format!("https://{id}.example.com/v1"),
            known_public_key: "k".to_owned(),
            priority,
            enabled: true,
        }
    }

    #[test]
    fn build_fetch_set_appends_synthetic_custom_entries() {
        let configured = vec![configured_server("s1", 10)];
        let customs = vec![
            "file:///mnt/data/local.json".to_owned(),
            "https://cache.example.com/v1".to_owned(),
        ];

        let set = build_fetch_set(configured, &customs);

        assert_eq!(set.len(), 3);
        assert_eq!(set[0].id, "s1");

        assert_eq!(set[1].id, "custom-0");
        assert_eq!(set[1].base_url, "file:///mnt/data/local.json");
        assert_eq!(set[1].priority, 0);
        assert_eq!(set[1].server_type, "custom");
        assert!(set[1].enabled);

        assert_eq!(set[2].id, "custom-1");
        assert_eq!(set[2].base_url, "https://cache.example.com/v1");
        assert_eq!(set[2].priority, 0);
    }

    #[test]
    fn ensure_fetchable_errors_when_no_enabled_entries() {
        assert!(
            ensure_fetchable(&[]).is_err(),
            "an empty fetch set must be a fatal error"
        );

        let mut disabled = configured_server("s1", 10);
        disabled.enabled = false;
        assert!(
            ensure_fetchable(&[disabled]).is_err(),
            "an all-disabled fetch set must be a fatal error"
        );
    }

    #[test]
    fn ensure_fetchable_ok_with_one_enabled_entry() {
        ensure_fetchable(&[configured_server("s1", 10)])
            .expect("BUG: one enabled entry should be fetchable");
    }

    #[tokio::test]
    async fn only_index_fetch_does_not_follow_federated_indexes() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");

        let child_path = dir.path().join("child.json");
        let child_ref = format!("file://{}", child_path.display());
        std::fs::write(
            &child_path,
            r#"{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[{"name":"federated","version":"1.0.0","store_path":"/nix/store/federated"}]}"#,
        )
        .expect("BUG: write child index");

        let root_path = dir.path().join("root.json");
        std::fs::write(
            &root_path,
            format!(
                r#"{{"version":1,"provenance":null,"indexes":[{}],"caches":[],"packages":[{{"name":"pinned","version":"1.0.0","store_path":"/nix/store/pinned"}}]}}"#,
                serde_json::to_string(&child_ref).expect("BUG: serialize child reference")
            ),
        )
        .expect("BUG: write root index");

        let root_ref = format!("file://{}", root_path.display());
        let fetch_set = build_fetch_set(Vec::new(), &[root_ref]);
        let client = reqwest::Client::new();
        let merged = fetch_and_merge_primary_indexes(&client, &fetch_set)
            .await
            .expect("BUG: primary-only fetch should merge the explicit index");

        assert_eq!(merged.by_name.get("pinned").map(Vec::len), Some(1));
        assert!(
            !merged.by_name.contains_key("federated"),
            "only-indexes must not follow indexes[] children"
        );
    }

    #[test]
    fn format_stale_warning_none_when_empty() {
        assert_eq!(format_stale_warning(&[]), None);
    }

    #[test]
    fn format_stale_warning_lists_sorted_packages() {
        let stale = vec![
            PackageVersion {
                name: "zeta".to_owned(),
                version: "2.0.0".to_owned(),
            },
            PackageVersion {
                name: "alpha".to_owned(),
                version: "1.0.0".to_owned(),
            },
        ];
        let warning = format_stale_warning(&stale).expect("BUG: non-empty stale set must warn");
        assert_eq!(
            warning,
            "Warning: 2 package(s) kept at installed version (absent from the indexes or no in-pin upgrade available):\n  ! alpha 1.0.0\n  ! zeta 2.0.0"
        );
    }

    #[tokio::test]
    async fn upgrade_cycle_changes_in_pin_and_reports_stale() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");
        let index_path = dir.path().join("index.json");
        std::fs::write(
            &index_path,
            r#"{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[{"name":"clock","version":"1.1.0","store_path":"/nix/store/clock-1.1.0"}]}"#,
        )
        .expect("BUG: write index");

        let file_server = ServerEntry {
            id: "braiins".to_owned(),
            server_type: "mirror".to_owned(),
            base_url: format!("file://{}", index_path.display()),
            known_public_key: "k".to_owned(),
            priority: 10,
            enabled: true,
        };

        let client = reqwest::Client::new();
        let merged = bmc_nix::index::fetch_and_merge_indexes(&client, &[file_server])
            .await
            .expect("BUG: merge should succeed");

        let mut packages = std::collections::BTreeMap::new();
        packages.insert(
            "clock".to_owned(),
            bmc_nix::types::ManifestPackage {
                version: "1.0.0".to_owned(),
                store_path: "/nix/store/clock-1.0.0".to_owned(),
                category: None,
                description: None,
                upgrade_strategy: None,
                install_strategy: None,
                installed_by: bmc_nix::types::InstalledBy::System,
                installed_from: "braiins".to_owned(),
                pinned: Some("^1.0.0".to_owned()),
            },
        );
        packages.insert(
            "ghost".to_owned(),
            bmc_nix::types::ManifestPackage {
                version: "3.0.0".to_owned(),
                store_path: "/nix/store/ghost-3.0.0".to_owned(),
                category: None,
                description: None,
                upgrade_strategy: None,
                install_strategy: None,
                // A missing system package is a hard error; only user
                // packages go stale when absent from every index.
                installed_by: bmc_nix::types::InstalledBy::User,
                installed_from: "braiins".to_owned(),
                pinned: None,
            },
        );
        let base = Manifest { packages };

        let plan = bmc_nix::manifest::compute_upgrade_plan(&base, Some(&merged), &[], &[])
            .expect("BUG: plan should compute");

        assert_eq!(plan.changed.len(), 1, "clock should change within its pin");
        assert_eq!(plan.changed[0].name, "clock");
        assert_eq!(plan.changed[0].from_version, "1.0.0");
        assert_eq!(plan.changed[0].to_version, "1.1.0");

        assert_eq!(plan.stale.len(), 1, "ghost is absent from the index");
        assert_eq!(plan.stale[0].name, "ghost");
        assert_eq!(plan.stale[0].version, "3.0.0");
    }

    #[tokio::test]
    async fn custom_index_wins_version_tie_against_configured_server() {
        let dir = tempfile::tempdir().expect("BUG: temp dir");

        let configured_path = dir.path().join("configured.json");
        std::fs::write(
            &configured_path,
            r#"{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[{"name":"clock","version":"1.0.0","store_path":"/nix/store/configured-clock"}]}"#,
        )
        .expect("BUG: write configured index");

        let custom_path = dir.path().join("custom.json");
        std::fs::write(
            &custom_path,
            r#"{"version":1,"provenance":null,"indexes":[],"caches":[],"packages":[{"name":"clock","version":"1.0.0","store_path":"/nix/store/custom-clock"}]}"#,
        )
        .expect("BUG: write custom index");

        let configured = vec![ServerEntry {
            id: "braiins".to_owned(),
            server_type: "mirror".to_owned(),
            base_url: format!("file://{}", configured_path.display()),
            known_public_key: "k".to_owned(),
            priority: 10,
            enabled: true,
        }];
        let customs = vec![format!("file://{}", custom_path.display())];

        let fetch_set = build_fetch_set(configured, &customs);
        let client = reqwest::Client::new();
        let merged = bmc_nix::index::fetch_and_merge_indexes(&client, &fetch_set)
            .await
            .expect("BUG: merge should succeed");

        let current = bmc_nix::types::ManifestPackage {
            version: "0.9.0".to_owned(),
            store_path: "/nix/store/current-clock".to_owned(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: bmc_nix::types::InstalledBy::System,
            installed_from: "local".to_owned(),
            pinned: None,
        };
        let resolved = bmc_nix::index::resolve_installed_package(&merged, "clock", &current)
            .expect("BUG: clock should resolve");

        assert_eq!(resolved.installed_from, "custom-0");
        assert_eq!(resolved.store_path, "/nix/store/custom-clock");
    }
}

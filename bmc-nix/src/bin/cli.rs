// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use bmc_nix::manifest;
use bmc_nix::types::{
    BaseSelector, GcConfig, InstallResult, Manifest, PackageChange, PackageVersion, ServersConfig,
};
use bmc_nix::upgrade::ActivationMode;
use clap::{Parser, Subcommand};

#[path = "cli/progress.rs"]
mod progress;

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

    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        base_manifest,
        None,
        &add_packages,
        &[],
        activation_mode_from_no_activate(no_activate),
        None,
        None,
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
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&profile_dir)?;

    let base_manifest = resolve_base(&profile_dir, &base)?;

    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        base_manifest,
        None,
        &[],
        &names,
        activation_mode_from_no_activate(no_activate),
        None,
        None,
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
) -> anyhow::Result<()> {
    let index_content = std::fs::read_to_string(&index)?;
    let package_index: bmc_nix::types::PackageIndex = serde_json::from_str(&index_content)?;
    let packages = bmc_nix::index::resolve_all_from_index(&package_index);

    std::fs::create_dir_all(&profile_dir)?;

    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        Some(Manifest::default()),
        None,
        &packages,
        &[],
        activation_mode_from_no_activate(no_activate),
        None,
        None,
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

/// Load the server registry from `path`.
///
/// A missing or unparseable file is fatal: init consumes the config
/// provisioning already installed and never repairs or falls back.
fn load_servers_config(path: &Path) -> anyhow::Result<ServersConfig> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read servers config at {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse servers config at {}", path.display()))
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
    bmc_nix::gc::collect_garbage(&bmc_nix::store::TokioCommandRunner, None).await?;
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "CLI dispatch — one match arm per subcommand"
)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

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
}

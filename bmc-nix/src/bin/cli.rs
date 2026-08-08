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

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use bmc_nix::activation::{self, ActivationOutcome, GenerationSelector};
use bmc_nix::index::{AD_HOC_INDEX_PRIORITY, AdHocIndexRef};
use bmc_nix::manifest;
use bmc_nix::mount::{self, MountOutcome};
use bmc_nix::registration::OtherServers;
use bmc_nix::types::{
    BaseSelector, FetchedIndex, GcConfig, InstallResult, Manifest, MergedIndex, PackageChange,
    PackageVersion, ServerEntry, ServerSource,
};
use bmc_nix::upgrade::ActivationMode;
use clap::{Parser, Subcommand};

#[path = "cli/progress.rs"]
mod progress;

/// Per-request timeout for fetching upgrade indexes over HTTP.
const INDEX_FETCH_TIMEOUT_SECS: u64 = 30;

const LOG_FILE: &str = "/var/log/bmc/bmc-nix-cli.log";
const CLI_DIAGNOSTIC_TARGET: &str = "bmc_nix_cli_diagnostic";

/// File present only on a real device. Its absence marks a build-sandbox or
/// host invocation, where writing under `/var/log/bmc` would be an impurity.
const DEVICE_MARKER: &str = "/etc/bos_version";

/// Log a human-readable diff of an `InstallResult` to the CLI diagnostic target.
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
/// Logs `Profile unchanged.` when the diff is empty.
///
/// A garbage-collection failure is reported as a warning: the profile
/// change itself already succeeded, only the post-activation cleanup did
/// not.
fn print_profile_diff(result: &InstallResult) {
    if let Err(err) = &result.gc {
        tracing::warn!(
            target: CLI_DIAGNOSTIC_TARGET,
            "profile updated but garbage collection failed: {err}"
        );
    }

    if result.added.is_empty() && result.removed.is_empty() && result.changed.is_empty() {
        tracing::info!(target: CLI_DIAGNOSTIC_TARGET, "Profile unchanged.");
        return;
    }

    tracing::info!(
        target: CLI_DIAGNOSTIC_TARGET,
        "Profile change: +{} added, -{} removed, {} changed",
        result.added.len(),
        result.removed.len(),
        result.changed.len(),
    );

    let mut added: Vec<&PackageVersion> = result.added.iter().collect();
    added.sort_by(|a, b| a.name.cmp(&b.name));
    for pv in added {
        tracing::info!(target: CLI_DIAGNOSTIC_TARGET, "  + {} {}", pv.name, pv.version);
    }

    let mut removed: Vec<&PackageVersion> = result.removed.iter().collect();
    removed.sort_by(|a, b| a.name.cmp(&b.name));
    for pv in removed {
        tracing::info!(target: CLI_DIAGNOSTIC_TARGET, "  - {} {}", pv.name, pv.version);
    }

    let mut changed: Vec<&PackageChange> = result.changed.iter().collect();
    changed.sort_by(|a, b| a.name.cmp(&b.name));
    for ch in changed {
        if ch.from_version == ch.to_version {
            tracing::info!(
                target: CLI_DIAGNOSTIC_TARGET,
                "  ~ {}: {} (store path changed)",
                ch.name,
                ch.from_version
            );
        } else {
            tracing::info!(
                target: CLI_DIAGNOSTIC_TARGET,
                "  ~ {}: {} -> {}",
                ch.name,
                ch.from_version,
                ch.to_version
            );
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

/// Output format for a command's stdout result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable `name  version` table.
    #[default]
    Human,
    /// Machine-readable JSON.
    Json,
}

/// Render a profile's installed packages, sorted by name.
///
/// With [`OutputFormat::Json`], emits `{"packages":[{"name","version","category"}]}`
/// for machine consumers; otherwise a `name  version` table per line.
fn render_package_list(manifest: &Manifest, format: OutputFormat) -> String {
    use std::fmt::Write as _;
    let mut names: Vec<&String> = manifest.packages.keys().collect();
    names.sort();
    if matches!(format, OutputFormat::Json) {
        let entries: Vec<serde_json::Value> = names
            .iter()
            .map(|name| {
                let pkg = &manifest.packages[*name];
                serde_json::json!({
                    "name": name,
                    "version": pkg.version,
                    "category": pkg.category,
                })
            })
            .collect();
        return serde_json::to_string_pretty(&serde_json::json!({ "packages": entries }))
            .expect("BUG: package list serializes");
    }
    let mut out = String::new();
    for name in names {
        let pkg = &manifest.packages[name];
        let _ = writeln!(out, "{name}  {}", pkg.version);
    }
    out
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

        /// Package name to record as `installed_by: system` in the
        /// minted manifest (repeatable). A system package missing from
        /// every index aborts later upgrades; all other packages are
        /// recorded as user-installed.
        #[arg(long = "system-package", required = true)]
        system_packages: Vec<String>,

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

        /// Package name to record as `installed_by: system` in the
        /// minted manifest (repeatable). A system package missing from
        /// every index aborts later upgrades; all other packages are
        /// recorded as user-installed.
        #[arg(long = "system-package", required = true)]
        system_packages: Vec<String>,

        #[command(flatten)]
        common: ProfileCommonArgs,
    },

    /// Upgrade installed packages against the configured indexes
    Upgrade {
        /// Path to the server registry
        /// (default: /etc/nix-upgrade/servers.json).
        #[arg(long)]
        servers_config: Option<PathBuf>,

        /// Ad-hoc index reference (repeatable), optionally `ID=`-prefixed
        /// (e.g. `forge=https://…`): an http(s) base URL, or a `file://`
        /// path to an index JSON. The id attributes installed packages
        /// (default `custom-<n>`). Highest precedence; a fetch failure
        /// aborts the run.
        #[arg(long = "index")]
        indexes: Vec<String>,

        /// Resolve exclusively against the --index references; do not read
        /// or consult the server registry.
        #[arg(
            long,
            action = clap::ArgAction::SetTrue,
            requires = "indexes",
            conflicts_with = "servers_config"
        )]
        only_indexes: bool,

        /// Read-only fallback used when the server registry file does
        /// not exist (default: the --servers-config path + ".default").
        #[arg(long, conflicts_with = "only_indexes")]
        default_servers_config: Option<PathBuf>,

        /// Base generation to diff against: `current` (default),
        /// `latest`, or a positive integer generation number.
        #[arg(long, default_value = "current")]
        base: BaseSelector,

        /// Firmware version that scopes feed-entry selection. Defaults to
        /// the contents of /etc/bos_version when registry resolution
        /// encounters a feed-linked server; required together with
        /// --next-boot.
        #[arg(long, value_parser = parse_bos_version)]
        firmware: Option<String>,

        /// Build the new generation but defer activation to the next boot:
        /// write the `next.<BOS_VERSION>` marker for the version supplied by
        /// --firmware instead of swapping `current`.
        #[arg(long, conflicts_with = "no_activate", requires = "firmware")]
        next_boot: bool,

        /// Package name to install during this upgrade (repeatable).
        /// Resolved against the same indexes as the upgrade.
        #[arg(long = "install")]
        install: Vec<String>,

        /// Read additional install package names from a pending-install
        /// handoff file (JSON `{"install": [..]}`), merged with --install.
        #[arg(long = "install-from")]
        install_from: Option<PathBuf>,

        #[command(flatten)]
        common: ProfileCommonArgs,
    },

    /// Initialize the Nix store from the configured factory tarball
    Init {
        /// Path to the server registry.
        #[arg(long, default_value = "/etc/nix-upgrade/servers.json")]
        servers_config: PathBuf,

        /// Read-only fallback used when the server registry file does
        /// not exist (default: the --servers-config path + ".default").
        #[arg(long)]
        default_servers_config: Option<PathBuf>,

        /// Data partition device to format and mount when needed.
        #[arg(long, default_value = "/dev/mmcblk0p4")]
        data_partition: PathBuf,

        /// Data partition mount point and staged extraction root.
        #[arg(long, default_value = "/mnt/data")]
        data_dir: PathBuf,

        /// Firmware version used to select the package-feed entry.
        /// Defaults to the contents of /etc/bos_version.
        #[arg(long, value_parser = parse_bos_version)]
        firmware: Option<String>,

        /// Directory for the downloaded factory tarball.
        #[arg(long, default_value = "/mnt/data")]
        download_dir: PathBuf,

        /// Replace an existing promoted store at <data-dir>/nix. The
        /// store is demoted after the downloaded tarball's signature
        /// is verified but before it is extracted: an init that fails
        /// during extraction leaves the device uninitialized, not on
        /// the old store.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        wipe: bool,

        /// Skip Ed25519 signature verification of the downloaded init
        /// tarball (development escape hatch; verification against the
        /// factory entry's known_public_key is on by default).
        #[arg(long, action = clap::ArgAction::SetTrue)]
        no_verify_signature: bool,

        /// Local factory tarball: skip the feed fetch and download and
        /// extract this file instead. The file is kept after
        /// extraction. Requires --profile-path.
        #[arg(
            long,
            requires = "profile_path",
            conflicts_with_all = [
                "servers_config",
                "default_servers_config",
                "firmware",
                "download_dir",
                "no_verify_signature",
            ]
        )]
        tarball: Option<PathBuf>,

        /// Profile path the tarball's pre-built generation was created
        /// for (the network path reads it from the feed entry).
        #[arg(long, requires = "tarball")]
        profile_path: Option<PathBuf>,
    },

    /// Fsck, format, and mount the data partition when needed
    PrepareDataPartition {
        /// Data partition device to format and mount when needed.
        #[arg(long, default_value = "/dev/mmcblk0p4")]
        data_partition: PathBuf,

        /// Data partition mount point.
        #[arg(long, default_value = "/mnt/data")]
        data_dir: PathBuf,
    },

    /// Check whether the promoted Nix store exists and is mounted at
    /// the Nix store mount point
    IsInitialized {
        /// Data partition mount point.
        #[arg(long, default_value = "/mnt/data")]
        data_dir: PathBuf,

        /// Nix store mount point the promoted store must be bound to.
        #[arg(long, default_value = "/nix")]
        nix_dir: PathBuf,
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

        /// Generation number to protect (repeatable). A non-empty list
        /// replaces the configured `protected_generations`.
        #[arg(long = "protected-generation")]
        protected_generations: Vec<usize>,
    },

    /// Bind-mount the persistent Nix store into `/nix`.
    Mount {
        /// Source directory (the persistent Nix store).
        #[arg(long, default_value = "/mnt/data/nix")]
        source: PathBuf,

        /// Target mount point.
        #[arg(long, default_value = "/nix")]
        target: PathBuf,
    },

    /// Activate a profile generation.
    ///
    /// Default target is `--generation current`. A failed activation of
    /// a non-current generation reverts to the current one.
    Activate {
        /// Profile-generation directory.
        #[arg(long, default_value = "/nix/var/nix/gcroots/profiles/bmc")]
        profile_dir: PathBuf,

        /// Which generation to activate: `current` (default), `latest`,
        /// `next` (consume the staged next profile), or a positive
        /// integer generation number.
        #[arg(long, value_parser = clap::value_parser!(GenerationSelector))]
        generation: Option<GenerationSelector>,

        /// File containing the running BOS version; selects the
        /// `next.<version>` marker for `--generation next`.
        #[arg(long, default_value = "/etc/bos_version")]
        bos_version_file: PathBuf,
    },

    /// List packages installed in the current profile
    ListPackages {
        /// Directory for the profile generations
        #[arg(long)]
        profile_dir: Option<PathBuf>,

        /// Output format
        #[arg(long, default_value_t = OutputFormat::Human, value_enum)]
        format: OutputFormat,
    },

    /// Register a package server, optionally with its binary-cache substituter.
    ///
    /// Inserts (or replaces by `id`) a server entry in `servers.json`; when
    /// the --cache-url/--cache-public-key pair is given, also appends the
    /// substituter plus its trusted key to `nix.conf`. Points a device at a
    /// developer machine for the upgrade test harness.
    #[command(group(
        clap::ArgGroup::new("source").required(true).multiple(false)
    ))]
    RegisterServer {
        /// Path to the server registry.
        #[arg(long, default_value = "/etc/nix-upgrade/servers.json")]
        servers_config: PathBuf,

        /// Path to the Nix configuration file.
        #[arg(long, default_value = "/etc/nix/nix.conf")]
        nix_conf: PathBuf,

        /// Unique server id; an existing entry with this id is replaced.
        #[arg(long)]
        id: String,

        /// Exact URL of the server's package feed document; exactly one
        /// of --feed-url/--index-url must be given.
        #[arg(long, group = "source", value_parser = parse_feed_url)]
        feed_url: Option<String>,

        /// Exact URL of the server's package index document; exactly one
        /// of --feed-url/--index-url must be given.
        #[arg(long, group = "source", value_parser = parse_index_url)]
        index_url: Option<String>,

        /// Base URL recorded on the factory entry when this registration
        /// bootstraps the registry or re-registers the bootstrapped
        /// factory id.
        #[arg(long, value_parser = parse_factory_base_url)]
        factory_base_url: Option<String>,

        /// Public key stored for future index verification; the device
        /// does not currently verify fetched indexes against it.
        #[arg(long)]
        index_public_key: String,

        /// URL of the binary-cache substituter. Optional as a pair with
        /// --cache-public-key; when absent, nix.conf is left untouched.
        #[arg(long, requires = "cache_public_key")]
        cache_url: Option<String>,

        /// Public key that signs the substituter's NARs.
        #[arg(long, requires = "cache_url")]
        cache_public_key: Option<String>,

        /// Resolution priority for the server entry.
        #[arg(long, default_value_t = 50)]
        priority: u32,

        /// Mark the server optional: a failed index fetch degrades to a
        /// warning instead of aborting the merge. Servers are required by
        /// default.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        optional: bool,

        /// Disable every other registered server so this one alone
        /// resolves upgrades. Priority cannot substitute: resolution
        /// ranks a candidate's version above its server's priority.
        /// The factory entry is left alone.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        exclusive: bool,
    },

    /// Remove every package server entry from the server registry.
    /// The factory entry and nix.conf are left untouched.
    ClearServers {
        /// Path to the runtime server registry.
        #[arg(long, default_value = "/etc/nix-upgrade/servers.json")]
        servers_config: PathBuf,

        /// Read-only fallback used when the server registry file does
        /// not exist (default: the --servers-config path + ".default").
        #[arg(long)]
        default_servers_config: Option<PathBuf>,
    },

    /// Sign an init tarball for a package-feed entry (nix-style
    /// `name:base64` Ed25519 signature over the tarball's SHA-256)
    SignInitTarball {
        /// Secret key file in `nix key generate-secret` format
        /// (`name:base64(seed ‖ public key)`).
        #[arg(long)]
        secret_key: PathBuf,

        /// Tarball to sign.
        tarball: PathBuf,
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

/// Default-config path: the explicit override when given, otherwise
/// the runtime config path with a literal ".default" suffix appended,
/// so a custom `--servers-config` keeps its default co-located
/// (`registry.conf` → `registry.conf.default`).
fn resolve_default_config_path(servers_config: &Path, explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| {
        let mut derived = servers_config.as_os_str().to_owned();
        derived.push(".default");
        PathBuf::from(derived)
    })
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
    protected_generations: Vec<usize>,
) {
    if let Some(value) = keep_generations {
        config.keep_generations = value;
    }
    if let Some(value) = keep_days {
        config.keep_days = Some(value);
    }
    if !protected_generations.is_empty() {
        config.protected_generations = protected_generations;
    }
}

/// Split an `--index` value into `(id, reference)`. An `ID=REFERENCE`
/// prefix names the entry explicitly — used so a firmware upgrade can
/// attribute packages to the same id as the configured server (e.g.
/// `forge`) and keep origin affinity across the flash. A bare reference
/// falls back to `custom-<n>` by flag order. The `=` is only an id
/// delimiter when the left side is a bare token (no `/` or `:`), so a URL
/// with a query string is never misread as `id=ref`.
fn parse_index_ref(value: &str, index: usize) -> (String, String) {
    if let Some((id, reference)) = value.split_once('=')
        && !id.is_empty()
        && !id.contains(['/', ':'])
    {
        return (id.to_owned(), reference.to_owned());
    }
    (format!("custom-{index}"), value.to_owned())
}

/// Build the ad-hoc fetch list: one reference per `--index` flag.
///
/// Ad-hoc references sit at priority 0 (highest precedence), so an ad-hoc
/// index wins a version tie against any configured server. Ids come from
/// each reference's optional `ID=` prefix, defaulting to `custom-<n>` by
/// flag order.
fn build_ad_hoc_refs(custom_indexes: &[String]) -> Vec<AdHocIndexRef> {
    custom_indexes
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let (id, reference) = parse_index_ref(value, i);
            AdHocIndexRef { id, reference }
        })
        .collect()
}

/// Fail when there is neither an enabled server nor an ad-hoc reference
/// to fetch from.
///
/// The shipped default `servers.json` carries an empty `servers` list, so
/// a plain `upgrade` with no `--index` would otherwise fetch nothing,
/// resolve every package as absent and mislabel them all as stale.
fn ensure_fetchable(servers: &[ServerEntry], ad_hoc: &[AdHocIndexRef]) -> anyhow::Result<()> {
    if servers.iter().any(|s| s.enabled) || !ad_hoc.is_empty() {
        return Ok(());
    }
    anyhow::bail!("no upgrade index configured: add a server to servers.json or pass --index <url>")
}

async fn fetch_and_merge_primary_indexes(
    client: &reqwest::Client,
    ad_hoc: &[AdHocIndexRef],
) -> Result<MergedIndex, bmc_nix::index::FetchIndexesError> {
    let mut fetched = Vec::with_capacity(ad_hoc.len());
    for reference in ad_hoc {
        let index = bmc_nix::index::fetch_index(client, &reference.reference).await?;
        fetched.push(FetchedIndex {
            server_id: reference.id.clone(),
            server_priority: AD_HOC_INDEX_PRIORITY,
            index,
        });
    }

    Ok(bmc_nix::index::merge_indexes(fetched))
}

/// Validate a BOS version at parse time: non-empty, no `/`, no
/// whitespace — it is embedded into the `next.<BOS_VERSION>` marker
/// file name and matched against package feed entries.
fn parse_bos_version(s: &str) -> Result<String, String> {
    if s.is_empty() || s.contains('/') || s.contains(char::is_whitespace) {
        return Err(format!("invalid BOS version '{s}'"));
    }
    Ok(s.to_owned())
}

fn parse_feed_url(s: &str) -> Result<String, String> {
    bmc_nix::types::validate_content_url("--feed-url", s)?;
    Ok(s.to_owned())
}

fn parse_index_url(s: &str) -> Result<String, String> {
    bmc_nix::types::validate_content_url("--index-url", s)?;
    Ok(s.to_owned())
}

fn parse_factory_base_url(s: &str) -> Result<String, String> {
    bmc_nix::types::validate_content_url("--factory-base-url", s)?;
    Ok(s.to_owned())
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
    system_packages: Vec<String>,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    no_activate: bool,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    let index_content = std::fs::read_to_string(&index)?;
    let package_index: bmc_nix::types::PackageIndex = serde_json::from_str(&index_content)?;
    let packages = bmc_nix::index::resolve_all_from_index(&package_index, &system_packages)?;

    std::fs::create_dir_all(&profile_dir)?;

    // An empty base plus every package as an addition mints a generation
    // holding exactly the index. The explicit base bypasses the no-op
    // short-circuit, so a generation is always built.
    let progress = progress::CliProgress::new(log_format);
    let result = bmc_nix::upgrade::apply_profile_change(
        &bmc_nix::store::Nix,
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
        .expect("BUG: build-profile always produces a generation");
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
            metadata: std::collections::BTreeMap::new(),
        })
        .collect();

    std::fs::create_dir_all(&profile_dir)?;

    let base_manifest = resolve_base(&profile_dir, &base)?;

    let progress = progress::CliProgress::new(log_format);
    let result = bmc_nix::upgrade::apply_profile_change(
        &bmc_nix::store::Nix,
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
        &bmc_nix::store::Nix,
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
    system_packages: Vec<String>,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    no_activate: bool,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    let index_content = std::fs::read_to_string(&index)?;
    let package_index: bmc_nix::types::PackageIndex = serde_json::from_str(&index_content)?;
    let packages = bmc_nix::index::resolve_all_from_index(&package_index, &system_packages)?;

    std::fs::create_dir_all(&profile_dir)?;

    let progress = progress::CliProgress::new(log_format);
    let result = bmc_nix::upgrade::apply_profile_change(
        &bmc_nix::store::Nix,
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
    default_servers_config: Option<PathBuf>,
    indexes: Vec<String>,
    only_indexes: bool,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    base: BaseSelector,
    firmware: Option<String>,
    no_activate: bool,
    next_boot: bool,
    install: Vec<String>,
    install_from: Option<PathBuf>,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    let activation_mode = if next_boot {
        ActivationMode::NextBoot {
            bos_version: firmware
                .clone()
                .expect("BUG: clap requires --firmware with --next-boot"),
        }
    } else {
        activation_mode_from_no_activate(no_activate)
    };
    let ad_hoc = build_ad_hoc_refs(&indexes);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(INDEX_FETCH_TIMEOUT_SECS))
        .build()
        .expect("BUG: reqwest client builder");
    let merged = if only_indexes {
        ensure_fetchable(&[], &ad_hoc)?;
        fetch_and_merge_primary_indexes(&client, &ad_hoc).await?
    } else {
        let servers_path =
            servers_config.unwrap_or_else(|| PathBuf::from("/etc/nix-upgrade/servers.json"));
        let default_path = resolve_default_config_path(&servers_path, default_servers_config);
        let config = bmc_nix::servers_config::load_servers_config(&servers_path, &default_path)?;
        ensure_fetchable(&config.servers, &ad_hoc)?;
        // /etc/bos_version is consulted only when a feed actually needs
        // scoping, so index-only registries keep working without it.
        let needs_scope = config
            .servers
            .iter()
            .any(|s| s.enabled && matches!(s.source, ServerSource::Feed { .. }));
        let scope = match (firmware, needs_scope) {
            (Some(version), _) => Some(version),
            (None, true) => Some(read_bos_version(Path::new("/etc/bos_version"))?),
            (None, false) => None,
        };
        bmc_nix::index::fetch_and_merge_indexes(&client, &config.servers, &ad_hoc, scope.as_deref())
            .await?
    };

    let mut install_names = install;
    if let Some(path) = install_from {
        let pending = bmc_nix::pending_install::read_pending_install(&path)?;
        install_names.extend(pending.install);
    }
    let install_packages: Vec<bmc_nix::types::ResolvedPackage> = install_names
        .iter()
        .map(|name| {
            bmc_nix::index::resolve_new_package(
                &merged,
                name,
                None,
                bmc_nix::types::InstalledBy::User,
            )
        })
        .collect::<Result<_, _>>()?;

    std::fs::create_dir_all(&profile_dir)?;
    let base_manifest = resolve_base(&profile_dir, &base)?;

    let progress = progress::CliProgress::new(log_format);
    let result = bmc_nix::upgrade::apply_profile_change(
        &bmc_nix::store::Nix,
        &profile_dir,
        base_manifest,
        Some(&merged),
        &install_packages,
        &[],
        activation_mode,
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

fn read_bos_version(path: &Path) -> anyhow::Result<String> {
    Ok(std::fs::read_to_string(path)
        .with_context(|| format!("failed to read BOS version from {}", path.display()))?
        .trim()
        .to_owned())
}

fn store_is_initialized(data_dir: &Path) -> bool {
    let nix_dir = data_dir.join("nix");
    let store_has_paths = std::fs::read_dir(nix_dir.join("store"))
        .is_ok_and(|mut entries| matches!(entries.next(), Some(Ok(_))));

    store_has_paths
        && nix_dir.join("var/nix/db/db.sqlite").is_file()
        && nix_dir.join("var/nix/gcroots/profiles/bmc").is_dir()
}

/// Shell `[ a -ef b ]` equivalent: the promoted store and the mount
/// point are the same filesystem object, i.e. the bind mount is in
/// place (and is not some foreign mount shadowing the target).
fn is_store_mounted(store_dir: &Path, nix_dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(store_dir), std::fs::metadata(nix_dir)) {
        (Ok(store), Ok(nix)) => store.dev() == nix.dev() && store.ino() == nix.ino(),
        _ => false,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatch — all args are required"
)]
async fn cmd_init(
    servers_config: PathBuf,
    default_servers_config: Option<PathBuf>,
    data_partition: PathBuf,
    data_dir: PathBuf,
    firmware: Option<String>,
    download_dir: PathBuf,
    wipe: bool,
    no_verify_signature: bool,
    tarball: Option<PathBuf>,
    profile_path: Option<PathBuf>,
    log_format: LogFormat,
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
        &bmc_nix::partition::read_proc_mounts()?,
        &bmc_nix::partition::read_proc_self_mountinfo()?,
    )
    .await?;

    // The firmware COMMAND distinguishes this no-op from a fresh
    // initialization by stdout: keep it empty here; a fresh init
    // prints the promoted profile path below.
    if store_is_initialized(&data_dir) && !wipe {
        tracing::info!("store already initialized");
        return Ok(());
    }

    let result = if let Some(tarball) = tarball {
        let profile_path = profile_path.expect("BUG: clap enforces --profile-path with --tarball");
        bmc_nix::store::init_store_from_tarball(&tarball, &profile_path, &data_dir, wipe).await?
    } else {
        let default_path = resolve_default_config_path(&servers_config, default_servers_config);
        let servers = bmc_nix::servers_config::load_servers_config(&servers_config, &default_path)?;
        let bos_version = match firmware {
            Some(version) => version,
            None => read_bos_version(Path::new("/etc/bos_version"))?,
        };
        let verification = if no_verify_signature {
            tracing::warn!(
                "init tarball signature verification disabled by --no-verify-signature; \
                 trusting the transport alone"
            );
            bmc_nix::store::SignatureVerification::Disabled
        } else {
            bmc_nix::store::SignatureVerification::Enabled {
                trusted_public_key: servers.factory.known_public_key.clone(),
            }
        };
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("BUG: failed to build HTTP client");
        let progress = progress::CliProgress::new(log_format);
        bmc_nix::store::init_store(
            &client,
            &servers.factory,
            &bos_version,
            &download_dir,
            &data_dir,
            wipe,
            &verification,
            Some(&progress),
        )
        .await?
    };

    println!("{}", result.profile_path.display());
    Ok(())
}

async fn cmd_gc(
    gc_config: Option<PathBuf>,
    profile_dir: PathBuf,
    keep_generations: Option<usize>,
    keep_days: Option<usize>,
    protected_generations: Vec<usize>,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    let config_path = gc_config.unwrap_or_else(|| PathBuf::from("/etc/nix-upgrade/gc.json"));
    let mut config = bmc_nix::gc::load_gc_config(&config_path)?;
    apply_gc_overrides(
        &mut config,
        keep_generations,
        keep_days,
        protected_generations,
    );

    let progress = progress::CliProgress::new(log_format);
    run_gc(
        &bmc_nix::store::Nix,
        &profile_dir,
        &config,
        forced_gc_request(),
        Some(&progress),
    )
    .await?;
    Ok(())
}

/// An operator asking for collection means it: wait for the profile and
/// sweep regardless of what cleanup removed.
fn forced_gc_request() -> bmc_nix::gc::GcRequest {
    bmc_nix::gc::GcRequest {
        on_busy: bmc_nix::gc::OnBusy::Wait,
        sweep: bmc_nix::gc::Sweep::Always,
    }
}

async fn run_gc(
    store: &impl bmc_nix::store::StoreOperations,
    profile_dir: &Path,
    config: &GcConfig,
    request: bmc_nix::gc::GcRequest,
    progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
) -> Result<bmc_nix::gc::ProfileGcOutcome, bmc_nix::gc::ProfileGcError> {
    bmc_nix::gc::collect_profile_garbage(store, profile_dir, config, request, progress).await
}

fn cmd_mount(source: &Path, target: &Path) -> anyhow::Result<()> {
    match mount::bind_mount_nix(source, target) {
        Ok(MountOutcome::Mounted) => {
            eprintln!("mount: bound {} -> {}", source.display(), target.display());
            Ok(())
        }
        Ok(MountOutcome::AlreadyMounted) => {
            eprintln!("mount: {} already mounted", target.display());
            Ok(())
        }
        Ok(MountOutcome::SourceMissing) => {
            eprintln!("mount: source {} does not exist", source.display());
            std::process::exit(1);
        }
        Err(err) => Err(anyhow::Error::new(err)),
    }
}

async fn cmd_activate(
    profile_dir: &Path,
    generation: Option<GenerationSelector>,
    bos_version_file: &Path,
) -> anyhow::Result<()> {
    let selector = generation.unwrap_or(GenerationSelector::Current);
    // A missing, unreadable, or empty version file must not fail the
    // boot path: without a version no marker can be matched and `next`
    // degrades to `current` inside `activate`. An empty version must
    // become `None` — as `Some("")` it would name the marker `next.`
    // and sweep every other marker as stale.
    let bos_version = if matches!(selector, GenerationSelector::Next) {
        read_bos_version(bos_version_file)
            .inspect_err(|err| eprintln!("activate: cannot read BOS version: {err:#}"))
            .ok()
            .filter(|version| !version.is_empty())
    } else {
        None
    };
    let outcome = activation::activate(profile_dir, selector, bos_version.as_deref()).await;
    match outcome {
        Ok(ActivationOutcome::Activated { generation, path }) => {
            eprintln!(
                "activate: activated generation {generation} at {}",
                path.display()
            );
            Ok(())
        }
        Ok(ActivationOutcome::Skipped) => {
            eprintln!("activate: no current profile and no generation links, skipping activation");
            Ok(())
        }
        Err(err) => Err(anyhow::Error::new(err)),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatch — mirrors the clap variant"
)]
fn cmd_register_server(
    servers_config: &Path,
    default_servers_config: &Path,
    nix_conf: &Path,
    id: String,
    source: ServerSource,
    factory_base_url: Option<&str>,
    index_public_key: String,
    cache_url: Option<&str>,
    cache_public_key: Option<&str>,
    priority: u32,
    optional: bool,
    other_servers: OtherServers,
) -> anyhow::Result<()> {
    let cache = match (cache_url, cache_public_key) {
        (Some(url), Some(key)) => Some((url, key)),
        (None, None) => None,
        _ => anyhow::bail!("--cache-url and --cache-public-key must be given together"),
    };
    anyhow::ensure!(!id.trim().is_empty(), "--id must not be empty");
    match &source {
        ServerSource::Feed { feed_url } => {
            bmc_nix::types::validate_content_url("--feed-url", feed_url)
                .map_err(anyhow::Error::msg)?;
        }
        ServerSource::Index { index_url } => {
            bmc_nix::types::validate_content_url("--index-url", index_url)
                .map_err(anyhow::Error::msg)?;
        }
    }
    if let Some(url) = factory_base_url {
        bmc_nix::types::validate_content_url("--factory-base-url", url)
            .map_err(anyhow::Error::msg)?;
    }
    anyhow::ensure!(
        !index_public_key.trim().is_empty(),
        "--index-public-key must not be empty"
    );
    if let Some((cache_url, cache_public_key)) = cache {
        anyhow::ensure!(
            cache_url.starts_with("https://")
                || cache_url.starts_with("http://")
                || cache_url.starts_with("file://"),
            "--cache-url must be an http(s):// or file:// URL, got '{cache_url}'"
        );
        anyhow::ensure!(
            !cache_public_key.trim().is_empty(),
            "--cache-public-key must not be empty"
        );
    }
    let entry = ServerEntry {
        id,
        source,
        known_public_key: index_public_key,
        priority,
        enabled: true,
        required: !optional,
    };
    let prepared = bmc_nix::registration::prepare_registration(
        servers_config,
        default_servers_config,
        entry,
        factory_base_url,
        other_servers,
    )?;
    if let Some((cache_url, cache_public_key)) = cache {
        // Register the substituter (nix.conf) before persisting the server
        // registry: if the process dies between the two writes, a server
        // missing from the registry is more benign than a registered server
        // whose binary cache is absent — the latter fetches an index it then
        // cannot realize. The registry content was validated up front, so a
        // bad config aborts before nix.conf is touched.
        bmc_nix::registration::register_substituter(nix_conf, cache_url, cache_public_key)?;
    }
    prepared.persist()?;
    Ok(())
}

fn cmd_clear_servers(servers_config: &Path, default_servers_config: &Path) -> anyhow::Result<()> {
    match bmc_nix::registration::prepare_clear_servers(servers_config, default_servers_config)? {
        Some(prepared) => {
            prepared.persist()?;
            println!(
                "cleared package server entries in {}",
                servers_config.display()
            );
        }
        None => println!("no servers configuration present; nothing to clear"),
    }
    Ok(())
}

fn cmd_list_packages(profile_dir: Option<PathBuf>, format: OutputFormat) -> anyhow::Result<()> {
    let profile_dir =
        profile_dir.unwrap_or_else(|| PathBuf::from("/nix/var/nix/gcroots/profiles/bmc"));
    let manifest = manifest::read_current_manifest(&profile_dir)?;
    print!("{}", render_package_list(&manifest, format));
    Ok(())
}

/// Sign `tarball` with the nix-format secret key at `secret_key`;
/// returns the `name:base64(signature)` feed-entry line.
fn sign_init_tarball(secret_key: &Path, tarball: &Path) -> anyhow::Result<String> {
    let key = std::fs::read_to_string(secret_key)
        .with_context(|| format!("reading secret key {}", secret_key.display()))?;
    let digest = sha256_file(tarball)?;
    Ok(bmc_nix::signature::sign(key.trim(), &digest)?)
}

/// Streaming SHA-256 of a file — the tarball never fits in memory on
/// principle, and the digest must match what the device-side verifier
/// computes over the downloaded bytes.
fn sha256_file(path: &Path) -> anyhow::Result<[u8; 32]> {
    use std::io::Read as _;

    let file =
        std::fs::File::open(path).with_context(|| format!("opening tarball {}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    // Heap-allocated so the 64 KiB buffer clears clippy's stack-array
    // threshold; the tarball is streamed either way.
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("reading tarball {}", path.display()))?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }
    Ok(context
        .finish()
        .as_ref()
        .try_into()
        .expect("BUG: SHA-256 digests are 32 bytes"))
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    // File logging writes under `/var/log/bmc` and takes a sidecar lock,
    // an on-device side effect. Off-device invocations log to the console
    // only, so the `build-profile` step of the Nix tarball build
    // writes nothing beyond the profile it is there to build.
    // `--help`/`--version` exit inside `Cli::parse()` above and reach neither init.
    //
    // The guard holds the sidecar flock; binding it here reserves
    // the single-writer claim for the whole process, failure report included.
    let log_guard = if Path::new(DEVICE_MARKER).exists() {
        Some(bmc_log::init_file_and_console(
            Path::new(LOG_FILE),
            CLI_DIAGNOSTIC_TARGET,
        ))
    } else {
        bmc_log::init_console();
        None
    };

    match run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            if log_guard.is_some() {
                // The diagnostic target is exempt from `RUST_LOG`,
                // so this reaches both the rotated log and the console.
                tracing::error!(target: CLI_DIAGNOSTIC_TARGET, error = ?err, "command failed");
            } else {
                // Console-only logging honours `RUST_LOG` and can drop
                // the event outright; a failure must never print nothing.
                eprintln!("Error: {err:?}");
            }
            // Exit 1 belongs to is-initialized's "store absent or incomplete"
            // answer, so runtime failures use 2 (clap's usage-error code):
            // the firmware COMMAND script must never mistake a broken run
            // for an absent store.
            std::process::ExitCode::from(2)
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "CLI dispatch — one match arm per subcommand"
)]
async fn run(cli: Cli) -> anyhow::Result<()> {
    let log_format = cli.log_format;

    match cli.command {
        Commands::BuildProfile {
            index,
            system_packages,
            profile_dir,
            hooks_dir,
            hooks_override_path,
            no_activate,
        } => {
            cmd_build_profile(
                index,
                system_packages,
                profile_dir,
                hooks_dir,
                hooks_override_path,
                no_activate,
                log_format,
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

        Commands::ResetProfile {
            index,
            system_packages,
            common,
        } => {
            cmd_reset_profile(
                index,
                system_packages,
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
            default_servers_config,
            indexes,
            only_indexes,
            base,
            firmware,
            next_boot,
            install,
            install_from,
            common,
        } => {
            cmd_upgrade(
                servers_config,
                default_servers_config,
                indexes,
                only_indexes,
                common.profile_dir,
                common.hooks_dir,
                common.hooks_override_path,
                base,
                firmware,
                common.no_activate,
                next_boot,
                install,
                install_from,
                log_format,
            )
            .await
        }

        Commands::Init {
            servers_config,
            default_servers_config,
            data_partition,
            data_dir,
            firmware,
            download_dir,
            wipe,
            no_verify_signature,
            tarball,
            profile_path,
        } => {
            cmd_init(
                servers_config,
                default_servers_config,
                data_partition,
                data_dir,
                firmware,
                download_dir,
                wipe,
                no_verify_signature,
                tarball,
                profile_path,
                log_format,
            )
            .await
        }

        Commands::PrepareDataPartition {
            data_partition,
            data_dir,
        } => {
            bmc_nix::partition::prepare_data_partition(
                &bmc_nix::store::TokioCommandRunner,
                &data_partition,
                &data_dir,
                &bmc_nix::partition::read_proc_mounts()?,
                &bmc_nix::partition::read_proc_self_mountinfo()?,
            )
            .await?;
            Ok(())
        }

        Commands::IsInitialized { data_dir, nix_dir } => {
            // The exit codes are a contract with the firmware
            // sysupgrade COMMAND script: 1 (store absent or incomplete)
            // and 3 (store present but not mounted) route into its
            // wipe-and-init branch. An unmounted store is inconsistent
            // state and is reinitialized for the incoming firmware.
            // Runtime failures anywhere in this binary exit 2 (see main)
            // so they can never masquerade as either answer.
            let store_dir = data_dir.join("nix");
            if !store_is_initialized(&data_dir) {
                std::process::exit(1)
            }
            if is_store_mounted(&store_dir, &nix_dir) {
                Ok(())
            } else {
                eprintln!(
                    "store present at {} but not mounted at {}",
                    store_dir.display(),
                    nix_dir.display()
                );
                std::process::exit(3)
            }
        }

        Commands::Gc {
            gc_config,
            profile_dir,
            keep_generations,
            keep_days,
            protected_generations,
        } => {
            cmd_gc(
                gc_config,
                profile_dir,
                keep_generations,
                keep_days,
                protected_generations,
                log_format,
            )
            .await
        }

        Commands::Mount { source, target } => cmd_mount(&source, &target),

        Commands::Activate {
            profile_dir,
            generation,
            bos_version_file,
        } => cmd_activate(&profile_dir, generation, &bos_version_file).await,

        Commands::ListPackages {
            profile_dir,
            format,
        } => cmd_list_packages(profile_dir, format),

        Commands::RegisterServer {
            servers_config,
            nix_conf,
            id,
            feed_url,
            index_url,
            factory_base_url,
            index_public_key,
            cache_url,
            cache_public_key,
            priority,
            optional,
            exclusive,
        } => {
            let source = match (feed_url, index_url) {
                (Some(feed_url), None) => ServerSource::Feed { feed_url },
                (None, Some(index_url)) => ServerSource::Index { index_url },
                (None, None) | (Some(_), Some(_)) => {
                    unreachable!("BUG: clap enforces exactly one of --feed-url/--index-url")
                }
            };
            cmd_register_server(
                &servers_config,
                &resolve_default_config_path(&servers_config, None),
                &nix_conf,
                id,
                source,
                factory_base_url.as_deref(),
                index_public_key,
                cache_url.as_deref(),
                cache_public_key.as_deref(),
                priority,
                optional,
                OtherServers::from(exclusive),
            )
        }
        Commands::ClearServers {
            servers_config,
            default_servers_config,
        } => {
            let default_servers_config =
                resolve_default_config_path(&servers_config, default_servers_config);
            cmd_clear_servers(&servers_config, &default_servers_config)
        }
        Commands::SignInitTarball {
            secret_key,
            tarball,
        } => {
            println!("{}", sign_init_tarball(&secret_key, &tarball)?);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Default)]
    struct RecordingGcStore {
        collect_calls: std::sync::atomic::AtomicUsize,
    }

    impl bmc_nix::store::StoreOperations for RecordingGcStore {
        async fn estimate_realization(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
        ) -> Result<bmc_nix::store::RealizeEstimate, bmc_nix::store::StorePathError> {
            unreachable!("BUG: gc command never estimates realization")
        }

        fn store_free_bytes(&self, _profile_dir: &std::path::Path) -> std::io::Result<u64> {
            unreachable!("BUG: gc command never measures free space")
        }

        async fn realize_store_paths(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
            _progress: Option<&dyn bmc_nix::store::RealizeProgress>,
        ) -> Result<(), bmc_nix::store::StorePathError> {
            unreachable!("BUG: gc command never realizes store paths")
        }

        async fn verify_store_paths(
            &self,
            _packages: &[bmc_nix::types::ResolvedPackage],
        ) -> Result<(), bmc_nix::store::StorePathError> {
            unreachable!("BUG: gc command never verifies store paths")
        }

        fn collect_garbage(
            &self,
            _progress: Option<&dyn bmc_nix::gc::CollectGarbageProgress>,
        ) -> impl std::future::Future<Output = Result<(), bmc_nix::gc::CollectGarbageError>> + Send
        {
            self.collect_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Ok(()) }
        }
    }

    #[tokio::test]
    async fn gc_command_collects_even_with_nothing_to_unroot() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        let store = RecordingGcStore::default();

        let outcome = run_gc(
            &store,
            &profile_dir,
            &GcConfig::default(),
            forced_gc_request(),
            None,
        )
        .await
        .expect("BUG: forced gc command succeeds");

        assert_eq!(outcome, bmc_nix::gc::ProfileGcOutcome::Collected);
        assert_eq!(
            store
                .collect_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1,
            "an operator asking for gc gets a store sweep regardless"
        );
    }

    #[tokio::test]
    async fn gc_command_ignores_a_disabling_configuration() {
        let tmp = tempfile::tempdir().expect("BUG: temp dir");
        let profile_dir = tmp.path().join("profile");
        std::fs::create_dir(&profile_dir).expect("BUG: create profile");
        let store = RecordingGcStore::default();
        let config = GcConfig {
            periodic: bmc_nix::types::PeriodicGcMode::Disabled,
            ..GcConfig::default()
        };

        let outcome = run_gc(&store, &profile_dir, &config, forced_gc_request(), None)
            .await
            .expect("BUG: forced gc command succeeds");

        assert_eq!(
            outcome,
            bmc_nix::gc::ProfileGcOutcome::Collected,
            "the toggle covers the periodic path, not an explicit request"
        );
    }

    #[test]
    fn upgrade_accepts_next_boot_flag() {
        let cli = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--index",
            "file:///tmp/index.json",
            "--firmware",
            "2026.07.1",
            "--next-boot",
        ])
        .expect("BUG: parse should succeed");

        let Commands::Upgrade {
            firmware,
            next_boot,
            common,
            ..
        } = cli.command
        else {
            panic!("BUG: parsed command must be upgrade");
        };
        assert!(next_boot, "next-boot activation must be recorded");
        assert_eq!(
            firmware.as_deref(),
            Some("2026.07.1"),
            "firmware scope must be recorded"
        );
        assert!(
            !common.no_activate,
            "next-boot must not imply the no-activate flag"
        );
    }

    #[test]
    fn upgrade_accepts_install_flags() {
        let cli = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--install",
            "widget-weather",
            "--install",
            "widget-ticker",
            "--install-from",
            "/tmp/pending.json",
        ])
        .expect("BUG: parse");
        let Commands::Upgrade {
            install,
            install_from,
            ..
        } = cli.command
        else {
            panic!("BUG: expected upgrade");
        };
        assert_eq!(
            install,
            vec!["widget-weather".to_owned(), "widget-ticker".to_owned()]
        );
        assert_eq!(
            install_from.as_deref(),
            Some(std::path::Path::new("/tmp/pending.json"))
        );
    }

    #[test]
    fn upgrade_rejects_next_boot_with_no_activate() {
        let err = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--index",
            "file:///tmp/index.json",
            "--firmware",
            "2026.07.1",
            "--next-boot",
            "--no-activate",
        ])
        .expect_err("BUG: next-boot conflicts with no-activate");

        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn upgrade_rejects_next_boot_without_firmware() {
        let err = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--index",
            "file:///tmp/index.json",
            "--next-boot",
        ])
        .expect_err("BUG: --next-boot requires --firmware at parse time");

        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn upgrade_rejects_malformed_firmware_at_parse() {
        let err = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--index",
            "file:///tmp/index.json",
            "--firmware",
            "2026 07.1",
        ])
        .expect_err("BUG: whitespace in a BOS version must fail at parse");

        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn register_server_requires_exactly_one_source_flag() {
        let base = [
            "bmc-nix-cli",
            "register-server",
            "--id",
            "dev",
            "--index-public-key",
            "index:KEY",
            "--cache-url",
            "https://cache.example.com",
            "--cache-public-key",
            "cache:KEY",
        ];

        let err = Cli::try_parse_from(base).expect_err("BUG: a source flag is required");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);

        let both = base
            .into_iter()
            .chain([
                "--feed-url",
                "https://dev.example.com/feed.json",
                "--index-url",
                "https://dev.example.com/index.json",
            ])
            .collect::<Vec<_>>();
        let err = Cli::try_parse_from(both).expect_err("BUG: the source flags are exclusive");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn register_server_rejects_relative_feed_url_at_parse() {
        let err = Cli::try_parse_from([
            "bmc-nix-cli",
            "register-server",
            "--id",
            "dev",
            "--feed-url",
            "feeds/feed.json",
            "--index-public-key",
            "index:KEY",
            "--cache-url",
            "https://cache.example.com",
            "--cache-public-key",
            "cache:KEY",
        ])
        .expect_err("BUG: a relative --feed-url must fail at parse");

        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn register_server_rejects_a_one_sided_cache_pair() {
        let base = [
            "bmc-nix-cli",
            "register-server",
            "--id",
            "forge",
            "--index-url",
            "https://forge.example/index.json",
            "--index-public-key",
            "forge:key",
        ];
        for lone in [
            ["--cache-url", "https://cache.example"],
            ["--cache-public-key", "cache:key"],
        ] {
            let args: Vec<&str> = base.iter().chain(lone.iter()).copied().collect();
            assert!(
                Cli::try_parse_from(args).is_err(),
                "a lone {} must be rejected at parse time",
                lone[0]
            );
        }
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
    fn upgrade_only_indexes_conflicts_with_servers_config() {
        let err = Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--only-indexes",
            "--index",
            "file:///tmp/index.json",
            "--servers-config",
            "/custom/servers.json",
        ])
        .expect_err("BUG: --only-indexes must reject --servers-config, not silently drop it");

        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn resolve_default_derives_by_literal_suffix() {
        assert_eq!(
            resolve_default_config_path(Path::new("/etc/nix-upgrade/servers.json"), None),
            PathBuf::from("/etc/nix-upgrade/servers.json.default")
        );
        assert_eq!(
            resolve_default_config_path(Path::new("/tmp/test/registry.conf"), None),
            PathBuf::from("/tmp/test/registry.conf.default")
        );
    }

    #[test]
    fn resolve_default_prefers_explicit_path() {
        assert_eq!(
            resolve_default_config_path(
                Path::new("/etc/nix-upgrade/servers.json"),
                Some(PathBuf::from("/run/sysupgrade/servers.json.default"))
            ),
            PathBuf::from("/run/sysupgrade/servers.json.default")
        );
    }

    #[test]
    fn upgrade_default_servers_config_conflicts_with_only_indexes() {
        Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--index",
            "file:///tmp/index.json",
            "--only-indexes",
            "--default-servers-config",
            "/tmp/servers.json.default",
        ])
        .expect_err("--default-servers-config must conflict with --only-indexes");
    }

    #[test]
    fn init_and_upgrade_accept_default_servers_config() {
        Cli::try_parse_from([
            "bmc-nix-cli",
            "init",
            "--default-servers-config",
            "/run/sysupgrade/servers.json.default",
        ])
        .expect("BUG: init must accept the flag");
        Cli::try_parse_from([
            "bmc-nix-cli",
            "upgrade",
            "--default-servers-config",
            "/run/sysupgrade/servers.json.default",
        ])
        .expect("BUG: upgrade must accept the flag");
    }

    #[test]
    fn register_server_leaves_nix_conf_untouched_when_prepare_fails() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let servers = tmp.path().join("servers.json");
        let nix_conf = tmp.path().join("nix.conf");
        std::fs::write(&servers, "{ corrupt").expect("BUG: write corrupt config");

        cmd_register_server(
            &servers,
            &resolve_default_config_path(&servers, None),
            &nix_conf,
            "dev".to_owned(),
            ServerSource::Index {
                index_url: "https://dev.example.com/v1/index.json".to_owned(),
            },
            None,
            "index:KEY".to_owned(),
            Some("https://cache.example.com"),
            Some("cache:KEY"),
            50,
            false,
            OtherServers::Keep,
        )
        .expect_err("prepare failure must abort the command");

        assert!(
            !nix_conf.exists(),
            "nix.conf must not be touched when prepare fails"
        );
    }

    #[tokio::test]
    async fn register_server_run_forwards_exclusive_to_registration() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let servers = tmp.path().join("servers.json");
        let nix_conf = tmp.path().join("nix.conf");
        std::fs::write(
            &servers,
            r#"{
                "factory": {
                    "id": "factory",
                    "base_url": "https://factory.example.com",
                    "known_public_key": "factory:KEY",
                    "priority": 0,
                    "enabled": true
                },
                "servers": [{
                    "id": "forge",
                    "index_url": "https://forge.example.com/index.json",
                    "known_public_key": "forge:KEY",
                    "priority": 50,
                    "enabled": true
                }]
            }"#,
        )
        .expect("BUG: write registry");
        let cli = Cli::try_parse_from([
            "bmc-nix-cli",
            "register-server",
            "--servers-config",
            servers.to_str().expect("BUG: temp path must be UTF-8"),
            "--nix-conf",
            nix_conf.to_str().expect("BUG: temp path must be UTF-8"),
            "--id",
            "dev",
            "--index-url",
            "https://dev.example.com/v1/index.json",
            "--index-public-key",
            "index:KEY",
            "--cache-url",
            "https://cache.example.com",
            "--cache-public-key",
            "cache:KEY",
            "--exclusive",
        ])
        .expect("BUG: register-server should parse");

        run(cli).await.expect("BUG: registration should succeed");

        let config: bmc_nix::types::ServersConfig = serde_json::from_str(
            &std::fs::read_to_string(servers).expect("BUG: read updated registry"),
        )
        .expect("BUG: updated registry must parse");
        let forge = config
            .servers
            .iter()
            .find(|server| server.id == "forge")
            .expect("BUG: registration must keep the existing server");
        assert!(
            !forge.enabled,
            "--exclusive must reach registration so production servers cannot decide the upgrade"
        );
    }

    #[test]
    fn register_without_the_cache_pair_skips_nix_conf() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let default = dir.path().join("servers.json.default");
        let nix_conf = dir.path().join("nix.conf");
        std::fs::write(
            &servers,
            r#"{"factory": {"id": "factory", "base_url": "https://factory.example",
                "known_public_key": "factory:key", "priority": 10, "enabled": true},
                "servers": []}"#,
        )
        .expect("BUG: write servers.json");

        cmd_register_server(
            &servers,
            &default,
            &nix_conf,
            "forge".to_owned(),
            ServerSource::Index {
                index_url: "https://forge.example/index.json".to_owned(),
            },
            None,
            "forge:key".to_owned(),
            None,
            None,
            50,
            false,
            OtherServers::Keep,
        )
        .expect("BUG: registration without a cache pair must succeed");

        assert!(!nix_conf.exists(), "no cache pair, no nix.conf write");
        let written: bmc_nix::types::ServersConfig =
            serde_json::from_str(&std::fs::read_to_string(&servers).expect("BUG: read back"))
                .expect("BUG: written config must parse");
        assert_eq!(written.servers.len(), 1);
    }

    #[test]
    fn register_with_the_cache_pair_still_writes_nix_conf() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let servers = dir.path().join("servers.json");
        let default = dir.path().join("servers.json.default");
        let nix_conf = dir.path().join("nix.conf");
        std::fs::write(
            &servers,
            r#"{"factory": {"id": "factory", "base_url": "https://factory.example",
                "known_public_key": "factory:key", "priority": 10, "enabled": true},
                "servers": []}"#,
        )
        .expect("BUG: write servers.json");

        cmd_register_server(
            &servers,
            &default,
            &nix_conf,
            "forge".to_owned(),
            ServerSource::Index {
                index_url: "https://forge.example/index.json".to_owned(),
            },
            None,
            "forge:key".to_owned(),
            Some("https://cache.example"),
            Some("cache:key"),
            50,
            false,
            OtherServers::Keep,
        )
        .expect("BUG: registration with a cache pair must succeed");

        let conf = std::fs::read_to_string(&nix_conf).expect("BUG: nix.conf must be written");
        assert!(conf.contains("https://cache.example"));
        assert!(conf.contains("cache:key"));
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
    fn prepare_data_partition_parses_with_defaults() {
        let cli = Cli::try_parse_from(["bmc-nix-cli", "prepare-data-partition"])
            .expect("BUG: prepare-data-partition should parse with defaults");

        let Commands::PrepareDataPartition {
            data_partition,
            data_dir,
        } = cli.command
        else {
            panic!("BUG: parsed command must be prepare-data-partition");
        };
        assert_eq!(data_partition, PathBuf::from("/dev/mmcblk0p4"));
        assert_eq!(data_dir, PathBuf::from("/mnt/data"));
    }

    #[test]
    fn is_initialized_defaults_to_data_dir() {
        let cli = Cli::try_parse_from(["bmc-nix-cli", "is-initialized"])
            .expect("BUG: is-initialized should parse with defaults");

        let Commands::IsInitialized { data_dir, nix_dir } = cli.command else {
            panic!("BUG: parsed command must be is-initialized");
        };
        assert_eq!(data_dir, PathBuf::from("/mnt/data"));
        assert_eq!(nix_dir, PathBuf::from("/nix"));
    }

    #[test]
    fn store_is_initialized_requires_store_database_and_profile() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        assert!(!store_is_initialized(tmp.path()));

        std::fs::create_dir_all(tmp.path().join("nix.tmp/nix")).expect("BUG: setup");
        assert!(
            !store_is_initialized(tmp.path()),
            "staged but unpromoted store must not count as initialized"
        );

        std::fs::write(tmp.path().join("nix"), "").expect("BUG: setup");
        assert!(
            !store_is_initialized(tmp.path()),
            "a non-directory nix path must not count as initialized"
        );
        std::fs::remove_file(tmp.path().join("nix")).expect("BUG: cleanup");

        std::fs::create_dir(tmp.path().join("nix")).expect("BUG: setup");
        assert!(
            !store_is_initialized(tmp.path()),
            "an empty promoted store must not count as initialized"
        );

        std::fs::create_dir_all(tmp.path().join("nix/store/package")).expect("BUG: setup");
        assert!(
            !store_is_initialized(tmp.path()),
            "store paths without the Nix database must not count as initialized"
        );

        let database = tmp.path().join("nix/var/nix/db/db.sqlite");
        std::fs::create_dir_all(database.parent().expect("BUG: database has a parent"))
            .expect("BUG: setup");
        std::fs::write(&database, "").expect("BUG: setup");
        assert!(
            !store_is_initialized(tmp.path()),
            "a store without the BMC profile directory must not count as initialized"
        );

        std::fs::create_dir_all(tmp.path().join("nix/var/nix/gcroots/profiles/bmc"))
            .expect("BUG: setup");
        assert!(
            store_is_initialized(tmp.path()),
            "a populated store with its database and BMC profile is initialized"
        );
    }

    #[test]
    fn is_store_mounted_requires_the_same_filesystem_object() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let store_dir = tmp.path().join("nix");
        std::fs::create_dir(&store_dir).expect("BUG: setup");

        // Same directory reached through two paths stands in for the
        // bind mount: identical device and inode, like `[ a -ef b ]`.
        let alias = tmp.path().join("alias");
        std::os::unix::fs::symlink(&store_dir, &alias).expect("BUG: setup");
        assert!(is_store_mounted(&store_dir, &alias));

        let other = tmp.path().join("other");
        std::fs::create_dir(&other).expect("BUG: setup");
        assert!(
            !is_store_mounted(&store_dir, &other),
            "a different directory on the mount point must not count as mounted"
        );

        assert!(
            !is_store_mounted(&store_dir, &tmp.path().join("missing")),
            "a missing mount point must not count as mounted"
        );
    }

    #[test]
    fn apply_gc_overrides_replaces_only_provided_fields() {
        let mut config = GcConfig {
            keep_generations: 3,
            keep_days: None,
            protected_generations: vec![1],
            ..GcConfig::default()
        };
        apply_gc_overrides(&mut config, Some(8), Some(30), vec![4, 6]);
        assert_eq!(config.keep_generations, 8);
        assert_eq!(config.keep_days, Some(30));
        assert_eq!(config.protected_generations, vec![4, 6]);
    }

    #[test]
    fn apply_gc_overrides_keeps_loaded_values_when_unset() {
        let mut config = GcConfig {
            keep_generations: 3,
            keep_days: Some(10),
            protected_generations: vec![1, 2],
            ..GcConfig::default()
        };
        apply_gc_overrides(&mut config, None, None, Vec::new());
        assert_eq!(config.keep_generations, 3);
        assert_eq!(config.keep_days, Some(10));
        assert_eq!(config.protected_generations, vec![1, 2]);
    }

    fn configured_server(id: &str, priority: u32) -> ServerEntry {
        ServerEntry {
            id: id.to_owned(),
            source: ServerSource::Index {
                index_url: format!("https://{id}.example.com/v1/nix-package-index.v1.json"),
            },
            known_public_key: "k".to_owned(),
            priority,
            enabled: true,
            required: true,
        }
    }

    #[test]
    fn build_ad_hoc_refs_numbers_bare_references() {
        let customs = vec![
            "file:///mnt/data/local.json".to_owned(),
            "https://cache.example.com/v1".to_owned(),
        ];

        let refs = build_ad_hoc_refs(&customs);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].id, "custom-0");
        assert_eq!(refs[0].reference, "file:///mnt/data/local.json");
        assert_eq!(refs[1].id, "custom-1");
        assert_eq!(refs[1].reference, "https://cache.example.com/v1");
    }

    #[test]
    fn build_ad_hoc_refs_honors_explicit_index_ids() {
        let customs = vec![
            "forge=https://cache.example.com/v1".to_owned(),
            "https://cache.example.com/v2?a=b".to_owned(),
        ];

        let refs = build_ad_hoc_refs(&customs);

        // An `ID=` prefix names the entry, so a firmware upgrade can
        // attribute packages to the configured server's id.
        assert_eq!(refs[0].id, "forge");
        assert_eq!(refs[0].reference, "https://cache.example.com/v1");
        // A bare reference keeps the `custom-<n>` fallback by flag order, and
        // an `=` inside the URL is not mistaken for an id delimiter.
        assert_eq!(refs[1].id, "custom-1");
        assert_eq!(refs[1].reference, "https://cache.example.com/v2?a=b");
    }

    #[test]
    fn ensure_fetchable_errors_when_no_enabled_entries() {
        assert!(
            ensure_fetchable(&[], &[]).is_err(),
            "an empty fetch set must be a fatal error"
        );

        let mut disabled = configured_server("s1", 10);
        disabled.enabled = false;
        assert!(
            ensure_fetchable(&[disabled], &[]).is_err(),
            "an all-disabled fetch set must be a fatal error"
        );
    }

    #[test]
    fn ensure_fetchable_ok_with_one_enabled_entry() {
        ensure_fetchable(&[configured_server("s1", 10)], &[])
            .expect("BUG: one enabled entry should be fetchable");
        ensure_fetchable(
            &[],
            &build_ad_hoc_refs(&["file:///tmp/index.json".to_owned()]),
        )
        .expect("BUG: one ad-hoc reference should be fetchable");
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
        let ad_hoc = build_ad_hoc_refs(&[root_ref]);
        let client = reqwest::Client::new();
        let merged = fetch_and_merge_primary_indexes(&client, &ad_hoc)
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
            id: "forge".to_owned(),
            source: ServerSource::Index {
                index_url: format!("file://{}", index_path.display()),
            },
            known_public_key: "k".to_owned(),
            priority: 10,
            enabled: true,
            required: true,
        };

        let client = reqwest::Client::new();
        let merged = bmc_nix::index::fetch_and_merge_indexes(&client, &[file_server], &[], None)
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
                installed_from: "forge".to_owned(),
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
                installed_from: "forge".to_owned(),
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
            id: "forge".to_owned(),
            source: ServerSource::Index {
                index_url: format!("file://{}", configured_path.display()),
            },
            known_public_key: "k".to_owned(),
            priority: 10,
            enabled: true,
            required: true,
        }];
        let customs = vec![format!("file://{}", custom_path.display())];

        let ad_hoc = build_ad_hoc_refs(&customs);
        let client = reqwest::Client::new();
        let merged = bmc_nix::index::fetch_and_merge_indexes(&client, &configured, &ad_hoc, None)
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

    #[test]
    fn renders_package_list_as_json_sorted() {
        // `ManifestPackage.installed_by`/`installed_from` have no serde
        // default, so a fixture omitting them panics.
        let manifest: Manifest = serde_json::from_str(
            r#"{"packages":{
                "widget-weather":{"version":"1.3.0","store_path":"/nix/store/w","category":"widget","installed_by":"user","installed_from":"srv"},
                "core":{"version":"2.0.0","store_path":"/nix/store/c","category":"system","installed_by":"system","installed_from":"srv"}
            }}"#,
        )
        .expect("BUG: parse manifest");
        let out = render_package_list(&manifest, OutputFormat::Json);
        let value: serde_json::Value = serde_json::from_str(&out).expect("BUG: json");
        let names: Vec<&str> = value["packages"]
            .as_array()
            .expect("BUG: array")
            .iter()
            .map(|p| p["name"].as_str().expect("BUG: name"))
            .collect();
        assert_eq!(names, vec!["core", "widget-weather"], "sorted by name");
    }

    #[test]
    fn list_packages_accepts_format_flag() {
        let cli = Cli::try_parse_from(["bmc-nix-cli", "list-packages", "--format", "json"])
            .expect("BUG: parse");
        assert!(matches!(
            cli.command,
            Commands::ListPackages {
                format: OutputFormat::Json,
                ..
            }
        ));
    }

    #[test]
    fn cli_activate_generation_accepts_next() {
        let cli = Cli::try_parse_from(["bmc-nix-cli", "activate", "--generation", "next"])
            .expect("BUG: --generation next should parse");
        let Commands::Activate { generation, .. } = cli.command else {
            panic!("BUG: parsed command must be activate");
        };
        assert!(matches!(generation, Some(GenerationSelector::Next)));
    }

    #[test]
    fn init_accepts_tarball_with_profile_path() {
        let cli = Cli::try_parse_from([
            "bmc-nix-cli",
            "init",
            "--tarball",
            "/tmp/t.tar.gz",
            "--profile-path",
            "/nix/var/nix/gcroots/profiles/bmc",
        ])
        .expect("BUG: --tarball with --profile-path should parse");
        let Commands::Init {
            tarball,
            profile_path,
            ..
        } = cli.command
        else {
            panic!("BUG: parsed command must be init");
        };
        assert!(tarball.is_some());
        assert!(profile_path.is_some());
    }

    #[test]
    fn init_tarball_and_profile_path_are_both_or_neither() {
        for partial in [
            vec!["bmc-nix-cli", "init", "--tarball", "/tmp/t.tar.gz"],
            vec![
                "bmc-nix-cli",
                "init",
                "--profile-path",
                "/nix/var/nix/gcroots/profiles/bmc",
            ],
        ] {
            assert!(
                Cli::try_parse_from(partial.clone()).is_err(),
                "{partial:?} must be rejected"
            );
        }
    }

    #[test]
    fn init_accepts_explicit_firmware() {
        let cli = Cli::try_parse_from(["bmc-nix-cli", "init", "--firmware", "2026-test-1"])
            .expect("BUG: init should accept an explicit firmware version");

        let Commands::Init { firmware, .. } = cli.command else {
            panic!("BUG: parsed command must be init");
        };
        assert_eq!(firmware.as_deref(), Some("2026-test-1"));
    }

    #[test]
    fn init_rejects_bos_version_file() {
        let error = Cli::try_parse_from([
            "bmc-nix-cli",
            "init",
            "--bos-version-file",
            "/tmp/bos_version",
        ])
        .expect_err("BUG: init must accept the firmware value, not a file path");

        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn init_tarball_conflicts_with_feed_only_flags() {
        for (flag, value) in [
            ("--servers-config", "/tmp/servers.json"),
            ("--default-servers-config", "/tmp/servers.json.default"),
            ("--firmware", "2026-test-1"),
            ("--download-dir", "/tmp/dl"),
        ] {
            let args = [
                "bmc-nix-cli",
                "init",
                "--tarball",
                "/tmp/t.tar.gz",
                "--profile-path",
                "/nix/var/nix/gcroots/profiles/bmc",
                flag,
                value,
            ];
            assert!(
                Cli::try_parse_from(args).is_err(),
                "{flag} must conflict with --tarball"
            );
        }
    }

    #[test]
    fn sign_init_tarball_parses() {
        let cli = Cli::parse_from([
            "bmc-nix-cli",
            "sign-init-tarball",
            "--secret-key",
            "/keys/e2e.secret",
            "/serve/tarballs/nix.tar.gz",
        ]);
        let Commands::SignInitTarball {
            secret_key,
            tarball,
        } = cli.command
        else {
            panic!("expected SignInitTarball, got {:?}", cli.command);
        };
        assert_eq!(secret_key, PathBuf::from("/keys/e2e.secret"));
        assert_eq!(tarball, PathBuf::from("/serve/tarballs/nix.tar.gz"));
    }

    #[test]
    fn sign_init_tarball_round_trips_with_verifier() {
        use base64::Engine as _;
        use ring::signature::KeyPair as _;

        let dir = tempfile::tempdir().expect("BUG: tempdir creation cannot fail in tests");
        let seed = [7_u8; 32];
        let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed)
            .expect("BUG: any 32-byte seed is a valid Ed25519 seed");
        let public = pair.public_key().as_ref().to_vec();
        let mut secret_bytes = seed.to_vec();
        secret_bytes.extend_from_slice(&public);
        let b64 = base64::engine::general_purpose::STANDARD;
        let secret_path = dir.path().join("key.secret");
        std::fs::write(
            &secret_path,
            format!("sysupgrade-e2e-1:{}\n", b64.encode(&secret_bytes)),
        )
        .expect("BUG: writing to a tempdir cannot fail");
        let tarball = dir.path().join("nix.tar.gz");
        std::fs::write(&tarball, b"tarball-bytes").expect("BUG: writing to a tempdir cannot fail");

        let line =
            sign_init_tarball(&secret_path, &tarball).expect("signing a valid input must work");

        let digest = sha256_file(&tarball).expect("hashing an existing file must work");
        let public_line = format!("sysupgrade-e2e-1:{}", b64.encode(&public));
        bmc_nix::signature::verify(&public_line, &digest, line.trim())
            .expect("the CLI's signature must verify against bmc_nix::signature::verify");
    }

    #[test]
    fn sign_init_tarball_rejects_malformed_secret_key() {
        let dir = tempfile::tempdir().expect("BUG: tempdir creation cannot fail in tests");
        let secret_path = dir.path().join("key.secret");
        std::fs::write(&secret_path, "not-a-key").expect("BUG: writing to a tempdir cannot fail");
        let tarball = dir.path().join("t.tar.gz");
        std::fs::write(&tarball, b"x").expect("BUG: writing to a tempdir cannot fail");
        sign_init_tarball(&secret_path, &tarball).expect_err("a malformed key must be rejected");
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

use bmc_nix::manifest;
use bmc_nix::types::{BaseSelector, InstallResult, Manifest, PackageChange, PackageVersion};
use clap::{Parser, Subcommand};

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
fn print_profile_diff(result: &InstallResult) {
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

/// Top-level CLI for bmc-nix profile management.
#[derive(Parser)]
#[command(name = "bmc-nix-cli")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Available subcommands.
#[derive(Subcommand)]
enum Commands {
    /// Build a profile from an index JSON file
    BuildProfile {
        /// Path to miniminer-index.json
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

        /// Directory for the profile generations
        #[arg(long, default_value = "/nix/var/nix/gcroots/profiles/bmc")]
        profile_dir: PathBuf,

        /// Name of the hooks directory inside the profile (default: "hooks")
        #[arg(long, default_value = "hooks")]
        hooks_dir: String,

        /// Override path for hook executables (for cross-compilation bootstrap)
        #[arg(long)]
        hooks_override_path: Option<PathBuf>,

        /// Base generation to diff against: `current` (default),
        /// `latest`, or a positive integer generation number.
        #[arg(long, default_value = "current")]
        base: BaseSelector,

        /// Build the generation but do not create/update the `current`
        /// symlink. Activation is the default.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        no_activate: bool,
    },

    /// Remove packages from a profile
    RemovePackages {
        /// Directory for the profile generations
        #[arg(long, default_value = "/nix/var/nix/gcroots/profiles/bmc")]
        profile_dir: PathBuf,

        /// Package names to remove
        #[arg(long = "name", required = true)]
        names: Vec<String>,

        /// Name of the hooks directory inside the profile (default: "hooks")
        #[arg(long, default_value = "hooks")]
        hooks_dir: String,

        /// Override path for hook executables (for cross-compilation bootstrap)
        #[arg(long)]
        hooks_override_path: Option<PathBuf>,

        /// Base generation to diff against: `current` (default),
        /// `latest`, or a positive integer generation number.
        #[arg(long, default_value = "current")]
        base: BaseSelector,

        /// Build the generation but do not create/update the `current`
        /// symlink. Activation is the default.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        no_activate: bool,
    },

    /// Reset profile from an index JSON (no manifest merging)
    ResetProfile {
        /// Path to miniminer-index.json
        #[arg(long)]
        index: PathBuf,

        /// Directory for the profile generations
        #[arg(long, default_value = "/nix/var/nix/gcroots/profiles/bmc")]
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
    let packages = bmc_nix::index::resolve_all_from_index(&package_index)?;

    std::fs::create_dir_all(&profile_dir)?;

    // Hold lock to prevent TOCTOU race on generation number
    let lock = bmc_nix::profile::lock_profile(&profile_dir).await?;
    let generation_number = bmc_nix::profile::next_generation_number(&profile_dir)?;
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
    let activate = !no_activate;

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
            cache_url: None,
            cache_name: "local".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: bmc_nix::types::InstalledBy::User,
            installed_from: "local".into(),
            pinned: bmc_nix::types::PinStrategy::None,
        })
        .collect();

    std::fs::create_dir_all(&profile_dir)?;

    let base_manifest = resolve_base(&profile_dir, &base)?;

    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        base_manifest,
        &add_packages,
        &[],
        activate,
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
    let activate = !no_activate;

    std::fs::create_dir_all(&profile_dir)?;

    let base_manifest = resolve_base(&profile_dir, &base)?;

    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        base_manifest,
        &[],
        &names,
        activate,
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
    let activate = !no_activate;

    let index_content = std::fs::read_to_string(&index)?;
    let package_index: bmc_nix::types::PackageIndex = serde_json::from_str(&index_content)?;
    let packages = bmc_nix::index::resolve_all_from_index(&package_index)?;

    std::fs::create_dir_all(&profile_dir)?;

    let result = bmc_nix::upgrade::apply_profile_change(
        &profile_dir,
        Some(Manifest::default()),
        &packages,
        &[],
        activate,
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
            profile_dir,
            hooks_dir,
            hooks_override_path,
            base,
            no_activate,
        } => {
            cmd_add_packages(
                name,
                version,
                store_path,
                profile_dir,
                hooks_dir,
                hooks_override_path,
                base,
                no_activate,
            )
            .await
        }

        Commands::RemovePackages {
            profile_dir,
            names,
            hooks_dir,
            hooks_override_path,
            base,
            no_activate,
        } => {
            cmd_remove_packages(
                profile_dir,
                names,
                hooks_dir,
                hooks_override_path,
                base,
                no_activate,
            )
            .await
        }

        Commands::ResetProfile {
            index,
            profile_dir,
            hooks_dir,
            hooks_override_path,
            no_activate,
        } => {
            cmd_reset_profile(
                index,
                profile_dir,
                hooks_dir,
                hooks_override_path,
                no_activate,
            )
            .await
        }
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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

        /// Activate the profile after building (create 'current' symlink)
        #[arg(long)]
        activate: bool,
    },

    /// Add one or more local packages to the current profile
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

        /// Activate the profile after building (create 'current' symlink)
        #[arg(long)]
        activate: bool,
    },

    /// Remove packages from the current profile
    RemovePackages {
        /// Directory for the profile generations
        #[arg(long, default_value = "/nix/var/nix/gcroots/profiles/bmc")]
        profile_dir: PathBuf,

        /// Package names to remove
        #[arg(long = "name", required = true)]
        names: Vec<String>,

        /// Activate the profile after building
        #[arg(long)]
        activate: bool,
    },

}

async fn cmd_build_profile(
    index: PathBuf,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    activate: bool,
) -> anyhow::Result<()> {
    let index_content = std::fs::read_to_string(&index)?;
    let package_index: bmc_nix::types::PackageIndex = serde_json::from_str(&index_content)?;
    let packages = bmc_nix::index::resolve_all_from_index(&package_index)?;

    std::fs::create_dir_all(&profile_dir)?;

    // Hold lock to prevent TOCTOU race on generation number
    let _lock = bmc_nix::profile::lock_profile(&profile_dir).await?;
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
        bmc_nix::profile::activate_profile(&profile_dir, generation.number, &generation.path)
            .await?;
    }

    println!("{}", generation.path.display());
    Ok(())
}

async fn cmd_add_packages(
    name: Vec<String>,
    version: Vec<String>,
    store_path: Vec<String>,
    profile_dir: PathBuf,
    activate: bool,
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

    let current_manifest = bmc_nix::manifest::read_current_manifest(&profile_dir);
    let plan =
        bmc_nix::manifest::compute_upgrade_plan(&current_manifest, None, &add_packages, &[])?;

    let result = bmc_nix::upgrade::apply_profile_change(
        None,
        &profile_dir,
        None,
        &plan,
        activate,
        None,
        "hooks",
        None,
    )
    .await?;

    println!("{}", result.generation.path.display());
    Ok(())
}

async fn cmd_remove_packages(
    profile_dir: PathBuf,
    names: Vec<String>,
    activate: bool,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&profile_dir)?;

    let current_manifest = bmc_nix::manifest::read_current_manifest(&profile_dir);
    let plan = bmc_nix::manifest::compute_upgrade_plan(&current_manifest, None, &[], &names)?;

    let result = bmc_nix::upgrade::apply_profile_change(
        None,
        &profile_dir,
        None,
        &plan,
        activate,
        None,
        "hooks",
        None,
    )
    .await?;

    println!("{}", result.generation.path.display());
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
            activate,
        } => cmd_build_profile(index, profile_dir, hooks_dir, hooks_override_path, activate).await,

        Commands::AddPackages {
            name,
            version,
            store_path,
            profile_dir,
            activate,
        } => cmd_add_packages(name, version, store_path, profile_dir, activate).await,

        Commands::RemovePackages {
            profile_dir,
            names,
            activate,
        } => cmd_remove_packages(profile_dir, names, activate).await,
    }
}

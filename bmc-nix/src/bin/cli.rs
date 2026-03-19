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
        } => {
            // 1. Read and parse index
            let index_content = std::fs::read_to_string(&index)?;
            let package_index: bmc_nix::types::PackageIndex = serde_json::from_str(&index_content)?;

            // 2. Resolve packages
            let packages = bmc_nix::index::resolve_all_from_index(&package_index)?;

            // 3. Ensure profile dir exists
            std::fs::create_dir_all(&profile_dir)?;

            // 4. Build profile (hold lock to prevent TOCTOU race on generation number)
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

            // 5. Optionally activate
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
        }
    }

    Ok(())
}

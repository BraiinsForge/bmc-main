# `bmc-nix-cli` — CLI for Nix Profile Operations

## Placement

New `[[bin]]` target in `bmc-nix/Cargo.toml`, alongside the existing hook binaries. It reuses the library functions
directly.

```toml
[[bin]]
name = "bmc-nix-cli"
path = "src/bin/cli.rs"
```

## Purpose

Thin CLI wrapper around `bmc-nix` library functions. The primary consumer is the `mkTarball` Nix derivation, which needs
to build a profile at build time without the full daemon running.

Only subcommands needed for the initial tarball are defined here. More can be added later as needed (e.g., for
debugging, manual upgrades, or CI tooling).

## Subcommands

### `build-profile`

Build a profile from an index JSON file.

```
bmc-nix-cli build-profile \
  --index <path>         \  # path to miniminer-index.json
  --profile-dir <path>   \  # where to create the profile
                            # (e.g. /nix/var/nix/gcroots/profiles/bmc)
  --generation <number>     # generation number (1 for factory)
```

**What it does:**

1. Read and parse the index JSON from `--index`
2. Convert all packages in the index to `ResolvedPackage` entries (store paths are already in the index, no fetching
   needed since the packages are already in the local store during nix build)
3. Call `bmc_nix::profile::build_profile()` with the given `profile_dir`, `generation`, and resolved packages
4. Exit 0 on success, non-zero with error message on failure

**Example invocation (inside mkTarball derivation):**

```bash
${bmc-nix-cli}/bin/bmc-nix-cli build-profile \
  --index ${index}/miniminer-index.json \
  --profile-dir $rootDir/nix/var/nix/gcroots/profiles/bmc \
  --generation 1
```

This produces the symlink tree at `$rootDir/nix/var/nix/gcroots/profiles/bmc/1-link/` with all packages merged, hooks
executed, and manifest written.

## Implementation Sketch

```rust
// bmc-nix/src/bin/cli.rs

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "bmc-nix-cli")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

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

        /// Generation number to create
        #[arg(long)]
        generation: u32,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::BuildProfile {
            index,
            profile_dir,
            generation,
        } => {
            let index_content = tokio::fs::read_to_string(&index).await?;
            let package_index: bmc_nix::types::PackageIndex =
                serde_json::from_str(&index_content)?;

            let packages = bmc_nix::index::resolve_all(&package_index)?;

            tokio::fs::create_dir_all(&profile_dir).await?;
            bmc_nix::profile::build_profile(
                &profile_dir,
                generation,
                &packages,
                "hooks",
            )
            .await?;
        }
    }

    Ok(())
}
```

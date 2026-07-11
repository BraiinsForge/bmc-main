// Copyright (C) 2026  Braiins Systems s.r.o.

//! `bmc-netsim` entry point: run a blueprint of device instances
//! loaded from disk, or emit the blueprint JSON schema for authoring.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use bmc_netsim::blueprint::Blueprint;

#[derive(Parser, Debug)]
#[command(
    name = "bmc-netsim",
    about = "Generic mDNS + REST network-resource simulator"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Load a blueprint (JSON5) and run the simulated devices on the LAN.
    Run { blueprint: PathBuf },
    /// Print the blueprint JSON schema to stdout.
    Schema,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Schema => {
            let schema = schemars::schema_for!(Blueprint);
            println!("{}", serde_json::to_string_pretty(&schema)?);
        }
        Command::Run { blueprint } => {
            let text = std::fs::read_to_string(&blueprint)
                .with_context(|| format!("reading {}", blueprint.display()))?;
            let blueprint: Blueprint = json5::from_str(&text)
                .with_context(|| format!("parsing {}", blueprint.display()))?;
            tracing::info!(instances = blueprint.instances.len(), "blueprint loaded");
            bmc_netsim::serve(blueprint).await?;
        }
    }
    Ok(())
}

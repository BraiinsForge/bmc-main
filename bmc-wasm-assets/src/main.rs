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

use std::fs;
use std::path::PathBuf;

use anyhow::{Result, ensure};
use bmc_wasm_assets::{contains_package_asset_section, extract_package_assets};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Extract {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        wasm_output: PathBuf,
        #[arg(long)]
        asset_root: PathBuf,
        #[arg(long)]
        artifact_root: Option<PathBuf>,
    },
    VerifyStripped {
        #[arg(long)]
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    match Arguments::parse().command {
        Command::Extract {
            input,
            wasm_output,
            asset_root,
            artifact_root,
        } => extract_package_assets(&input, artifact_root.as_deref(), &wasm_output, &asset_root),
        Command::VerifyStripped { input } => {
            let wasm = fs::read(&input)?;
            ensure!(
                !contains_package_asset_section(&wasm)?,
                "{} contains package asset records",
                input.display()
            );
            Ok(())
        }
    }
}

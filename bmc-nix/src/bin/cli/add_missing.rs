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

use std::path::PathBuf;

use anyhow::Context as _;
use bmc_nix::add_missing::AddMissingOutcome;
use bmc_nix::types::ResolvedPackage;

use super::{LogFormat, print_profile_diff, progress};

pub(super) async fn cmd_add_missing_packages(
    add_packages: Vec<ResolvedPackage>,
    if_current: Option<PathBuf>,
    profile_dir: PathBuf,
    hooks_dir: String,
    hooks_override_path: Option<PathBuf>,
    log_format: LogFormat,
) -> anyhow::Result<()> {
    let expected_current = if_current
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(bmc_nix::profile::parse_generation_link_name)
                .with_context(|| {
                    format!(
                        "--if-current {} is not a <N>-link generation path",
                        path.display()
                    )
                })
        })
        .transpose()?;

    std::fs::create_dir_all(&profile_dir)?;

    let progress = progress::CliProgress::new(log_format);
    let outcome = bmc_nix::add_missing::add_missing_packages(
        &bmc_nix::store::Nix,
        &profile_dir,
        &add_packages,
        expected_current,
        Some(&progress),
        &hooks_dir,
        hooks_override_path.as_deref(),
    )
    .await?;

    match outcome {
        AddMissingOutcome::Installed(result) => {
            print_profile_diff(&result);
            if let Some(generation) = result.generation {
                println!("{}", generation.path.display());
            }
        }
        AddMissingOutcome::AlreadyInstalled => println!("already installed"),
        AddMissingOutcome::NextBootStaged => println!("next-boot generation staged"),
        AddMissingOutcome::ProfileChanged {
            current: Some(current),
        } => println!("profile changed: current is generation {current}"),
        AddMissingOutcome::ProfileChanged { current: None } => {
            println!("profile changed: no current generation");
        }
    }
    Ok(())
}

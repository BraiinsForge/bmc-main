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

//! External tool resolution — shared by subcommands that shell out to
//! `odiff`, `ffmpeg`, etc.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

/// Find a tool binary: check PATH first, then try `nix build` fallback.
pub fn resolve_tool(name: &str, nix_pkg: &str) -> Result<PathBuf> {
    // Check PATH
    if let Some(path) = find_in_path(name) {
        return Ok(path);
    }

    // Try nix
    if find_in_path("nix").is_some() {
        let output = Command::new("nix")
            .args([
                "build",
                &format!("nixpkgs#{nix_pkg}"),
                "--print-out-paths",
                "--no-link",
            ])
            .output()
            .context("failed to run nix build")?;
        if output.status.success() {
            let store_path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let bin = PathBuf::from(store_path).join("bin").join(name);
            if bin.exists() {
                return Ok(bin);
            }
        }
    }

    bail!(
        "{name} not found in PATH and nix fallback failed.\n\
         Install with: nix profile install nixpkgs#{nix_pkg}"
    );
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

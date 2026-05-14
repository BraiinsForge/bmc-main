// Copyright (C) 2026  Braiins Systems s.r.o.

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

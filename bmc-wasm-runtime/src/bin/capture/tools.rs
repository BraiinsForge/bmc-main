// Copyright (C) 2026  Braiins Systems s.r.o.

//! External tool resolution — shared by subcommands that shell out to
//! `odiff`, `ffmpeg`, etc.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

/// Find a tool binary: check PATH first, then try `nix build` fallback.
pub fn resolve_tool(name: &str, nix_pkg: &str) -> Result<PathBuf> {
    // Check PATH
    if let Ok(path) = which(name) {
        return Ok(path);
    }

    // Try nix
    if which("nix").is_ok() {
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

fn which(name: &str) -> Result<PathBuf> {
    let output = Command::new("which")
        .arg(name)
        .output()
        .context("failed to run which")?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok(PathBuf::from(path))
    } else {
        bail!("{name} not found")
    }
}

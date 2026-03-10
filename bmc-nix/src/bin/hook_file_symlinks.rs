// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize)]
struct FileSymlinkDef {
    priority: u32,
    from: String,
    to: String,
}

/// Validate that a path string contains only characters safe for shell interpolation.
///
/// Allows alphanumerics, `.`, `_`, `-`, `/` (path separators), and `+` (common in
/// Nix store paths). Rejects shell metacharacters like `$`, `` ` ``, `"`, `'`, `;`, etc.
fn validate_shell_safe(value: &str, field_name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !value.is_empty(),
        "file-symlinks: {field_name} must not be empty"
    );
    anyhow::ensure!(
        value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '+')),
        "file-symlinks: {field_name} contains unsafe characters: {value:?}"
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let gen_path_str = std::env::var("PROFILE_NEW_GENERATION")
        .map_err(|_| anyhow::anyhow!("PROFILE_NEW_GENERATION environment variable must be set"))?;
    let gen_path = Path::new(&gen_path_str);
    let symlinks_dir = gen_path.join("file-symlinks");

    if !symlinks_dir.exists() {
        return Ok(());
    }

    // Read all JSON definitions
    let mut defs: Vec<FileSymlinkDef> = Vec::new();
    for entry in std::fs::read_dir(&symlinks_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let content = std::fs::read_to_string(&path)?;
            let def: FileSymlinkDef = serde_json::from_str(&content)?;

            // Validate fields before interpolating into shell script
            validate_shell_safe(&def.from, "from")?;
            validate_shell_safe(&def.to, "to")?;

            defs.push(def);
        }
    }

    if defs.is_empty() {
        return Ok(());
    }

    // Deduplicate by target path — higher priority number wins
    let mut by_target: BTreeMap<String, FileSymlinkDef> = BTreeMap::new();
    for def in defs {
        by_target
            .entry(def.to.clone())
            .and_modify(|existing| {
                if def.priority > existing.priority {
                    *existing = FileSymlinkDef {
                        priority: def.priority,
                        from: def.from.clone(),
                        to: def.to.clone(),
                    };
                }
            })
            .or_insert(def);
    }

    // Generate activation script under core/activation/scripts/
    // Numbered 60- to run after write-boundary (50-)
    let scripts_dir = gen_path.join("core/activation/scripts");
    std::fs::create_dir_all(&scripts_dir)?;

    let mut script = String::from("#!/bin/sh\nset -e\n");
    for def in by_target.values() {
        writeln!(
            script,
            "ln -sf \"$PROFILE_NEW_GENERATION/{}\" \"{}\"",
            def.from, def.to
        )
        .expect("BUG: write to String should never fail");
    }

    let script_path = scripts_dir.join("60-file-symlinks");
    std::fs::write(&script_path, &script)?;
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;

    Ok(())
}

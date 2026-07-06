// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::fmt::Write as _;
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
    run(gen_path)
}

fn run(gen_path: &Path) -> anyhow::Result<()> {
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

    // Generate the write-phase activation script under
    // core/activation/scripts/. It links $PROFILE_NEW_GENERATION paths
    // into place; the 060 prefix runs it in the write phase, before the
    // final 'current' commit.
    let mut script = String::from("#!/bin/sh\nset -e\n");
    for def in by_target.values() {
        writeln!(
            script,
            "ln -sf \"$PROFILE_NEW_GENERATION/{}\" \"{}\"",
            def.from, def.to
        )
        .expect("BUG: write to String should never fail");
    }

    bmc_nix::generation_path::write_generated_file(
        gen_path,
        Path::new("core/activation/scripts/060-file-symlinks"),
        script.as_bytes(),
        0o755,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn run_materializes_core_ancestor_before_writing_file_symlink_script() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let generation = tmp.path().join("generation");
        let store_core = tmp.path().join("store/core");
        let file_symlinks = generation.join("file-symlinks");
        std::fs::create_dir_all(store_core.join("activation/scripts"))
            .expect("BUG: create store scripts");
        std::fs::create_dir_all(&file_symlinks).expect("BUG: create file-symlinks");
        std::fs::write(
            file_symlinks.join("link.json"),
            r#"{"priority": 10, "from": "bin/source", "to": "/tmp/target"}"#,
        )
        .expect("BUG: write definition");
        std::fs::create_dir_all(&generation).expect("BUG: create generation");
        std::os::unix::fs::symlink(&store_core, generation.join("core"))
            .expect("BUG: symlink core");

        super::run(&generation).expect("BUG: file-symlinks hook should succeed");

        let script = generation.join("core/activation/scripts/060-file-symlinks");
        let meta = script
            .symlink_metadata()
            .expect("BUG: stat generated script");
        assert!(meta.is_file(), "script should be a generated regular file");
        assert!(
            !meta.file_type().is_symlink(),
            "script should not be a symlink into the store"
        );
        assert!(
            !store_core
                .join("activation/scripts/060-file-symlinks")
                .exists(),
            "store-backed scripts directory must not be modified"
        );
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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
    let mut defs: Vec<(PathBuf, FileSymlinkDef)> = Vec::new();
    for entry in std::fs::read_dir(&symlinks_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let content = std::fs::read_to_string(&path)?;
            let def: FileSymlinkDef = serde_json::from_str(&content)?;

            // Validate fields before interpolating into shell script
            validate_shell_safe(&def.from, "from")?;
            validate_shell_safe(&def.to, "to")?;
            // `to` is passed verbatim to `ln`: a relative value (including
            // one starting with '-', which `ln` parses as options despite
            // the quoting) must be rejected. Neither field may climb out
            // of its root via `..`.
            anyhow::ensure!(
                def.to.starts_with('/'),
                "file-symlinks: to must be an absolute path: {:?}",
                def.to
            );
            for (field_name, value) in [("from", &def.from), ("to", &def.to)] {
                anyhow::ensure!(
                    Path::new(value)
                        .components()
                        .all(|c| c != std::path::Component::ParentDir),
                    "file-symlinks: {field_name} must not contain '..': {value:?}"
                );
            }

            defs.push((path, def));
        }
    }

    if defs.is_empty() {
        return Ok(());
    }

    // Sort by definition file name so equal-priority ties resolve
    // deterministically instead of depending on read_dir order.
    defs.sort_by(|(a, _), (b, _)| a.file_name().cmp(&b.file_name()));

    // Deduplicate by target path — higher priority number wins
    let mut by_target: BTreeMap<String, FileSymlinkDef> = BTreeMap::new();
    for (_path, def) in defs {
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
    let mut script = String::from("#!/bin/sh\nset -e\n");
    for def in by_target.values() {
        writeln!(
            script,
            "ln -sfn \"$PROFILE_NEW_GENERATION/{}\" \"{}\"",
            def.from, def.to
        )
        .expect("BUG: write to String should never fail");
    }

    bmc_nix::generation_path::write_generated_file(
        gen_path,
        Path::new("core/activation/scripts/60-file-symlinks"),
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

        let script = generation.join("core/activation/scripts/60-file-symlinks");
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
                .join("activation/scripts/60-file-symlinks")
                .exists(),
            "store-backed scripts directory must not be modified"
        );
    }

    /// Set up a generation with a store-backed core directory and return
    /// the generation path plus the path where the generated script lands.
    fn setup_generation(tmp: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let generation = tmp.join("generation");
        let store_core = tmp.join("store/core");
        std::fs::create_dir_all(store_core.join("activation/scripts"))
            .expect("BUG: create store scripts");
        std::fs::create_dir_all(generation.join("file-symlinks"))
            .expect("BUG: create file-symlinks");
        std::os::unix::fs::symlink(&store_core, generation.join("core"))
            .expect("BUG: symlink core");
        let script = generation.join("core/activation/scripts/60-file-symlinks");
        (generation, script)
    }

    #[test]
    fn generated_script_uses_no_dereference_ln() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let (generation, script) = setup_generation(tmp.path());
        std::fs::write(
            generation.join("file-symlinks/link.json"),
            r#"{"priority": 10, "from": "bin/source", "to": "/tmp/target"}"#,
        )
        .expect("BUG: write definition");

        super::run(&generation).expect("BUG: file-symlinks hook should succeed");

        let content = std::fs::read_to_string(&script).expect("BUG: read generated script");
        assert!(
            content.contains("ln -sfn "),
            "generated script should use no-dereference ln, got:\n{content}"
        );
    }

    #[test]
    fn equal_priority_ties_resolve_by_definition_filename() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let (generation, script) = setup_generation(tmp.path());
        std::fs::write(
            generation.join("file-symlinks/a.json"),
            r#"{"priority": 10, "from": "bin/from-a", "to": "/tmp/target"}"#,
        )
        .expect("BUG: write a.json");
        std::fs::write(
            generation.join("file-symlinks/b.json"),
            r#"{"priority": 10, "from": "bin/from-b", "to": "/tmp/target"}"#,
        )
        .expect("BUG: write b.json");

        super::run(&generation).expect("BUG: file-symlinks hook should succeed");

        let content = std::fs::read_to_string(&script).expect("BUG: read generated script");
        assert!(
            content.contains("bin/from-a"),
            "a.json's `from` should win the equal-priority tie, got:\n{content}"
        );
        assert!(
            !content.contains("bin/from-b"),
            "b.json's `from` should not win, got:\n{content}"
        );
    }

    #[test]
    fn non_absolute_link_target_is_rejected() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let (generation, script) = setup_generation(tmp.path());
        // Quoting cannot stop `ln` from reading a leading '-' as options,
        // so a relative target must never reach the generated script.
        std::fs::write(
            generation.join("file-symlinks/bad.json"),
            r#"{"priority": 10, "from": "bin/source", "to": "-x"}"#,
        )
        .expect("BUG: write definition");

        super::run(&generation).expect_err("a relative `to` must be rejected");
        assert!(!script.exists(), "no script for a rejected definition");
    }

    #[test]
    fn parent_dir_components_are_rejected() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let (generation, script) = setup_generation(tmp.path());
        std::fs::write(
            generation.join("file-symlinks/escape.json"),
            r#"{"priority": 10, "from": "../../etc/passwd", "to": "/tmp/target"}"#,
        )
        .expect("BUG: write definition");

        super::run(&generation).expect_err("a `..` component must be rejected");
        assert!(!script.exists(), "no script for a rejected definition");
    }
}

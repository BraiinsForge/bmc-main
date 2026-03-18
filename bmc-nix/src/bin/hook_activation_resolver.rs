// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let gen_path_str = std::env::var("PROFILE_NEW_GENERATION")
        .map_err(|_| anyhow::anyhow!("PROFILE_NEW_GENERATION environment variable must be set"))?;
    let gen_path = Path::new(&gen_path_str);
    let scripts_dir = gen_path.join("core/activation/scripts");

    if !scripts_dir.exists() {
        return Ok(());
    }

    // Collect script names, validating they contain only safe characters
    let mut scripts: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&scripts_dir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "activation script filename is not valid UTF-8: {:?}",
                    entry.file_name()
                )
            })?
            .to_owned();

        anyhow::ensure!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'),
            "activation script name contains unsafe characters: {name:?}"
        );

        scripts.push(name);
    }

    if scripts.is_empty() {
        return Ok(());
    }

    // Sort alphanumerically (lexicographic)
    scripts.sort();

    // Generate entrypoint script that calls each activation script in order.
    // The entrypoint defaults PROFILE_NEW_GENERATION from its own path when
    // not set (e.g., called directly at boot by the init service).
    // It lives at <gen_path>/core/activation/entrypoint, so the generation
    // path is two directories up from its location.
    let activation_dir = gen_path.join("core/activation");
    std::fs::create_dir_all(&activation_dir)?;

    let mut entrypoint = String::new();
    writeln!(entrypoint, "#!/bin/sh").expect("BUG: write to String should never fail");
    writeln!(entrypoint, "set -e").expect("BUG: write to String should never fail");
    writeln!(
        entrypoint,
        r#"ENTRYPOINT_DIR="$(cd "$(dirname "$0")" && pwd -P)""#
    )
    .expect("BUG: write to String should never fail");
    writeln!(
        entrypoint,
        r#"if [ -z "$PROFILE_NEW_GENERATION" ]; then
  PROFILE_NEW_GENERATION="$(dirname "$(dirname "$ENTRYPOINT_DIR")")"
fi
export PROFILE_NEW_GENERATION"#
    )
    .expect("BUG: write to String should never fail");
    writeln!(entrypoint, r#"SCRIPTS_DIR="$ENTRYPOINT_DIR/scripts""#)
        .expect("BUG: write to String should never fail");

    for script in &scripts {
        writeln!(entrypoint, "\"$SCRIPTS_DIR/{script}\"")
            .expect("BUG: write to String should never fail");
    }

    let entrypoint_path = activation_dir.join("entrypoint");
    std::fs::write(&entrypoint_path, &entrypoint)?;
    std::fs::set_permissions(&entrypoint_path, std::fs::Permissions::from_mode(0o755))?;

    Ok(())
}

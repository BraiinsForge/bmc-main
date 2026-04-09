// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

fn write_entrypoint(scripts: &[String]) -> String {
    let mut entrypoint = String::new();
    writeln!(entrypoint, "#!/bin/sh").expect("BUG: write to String should never fail");
    writeln!(entrypoint, "set -e").expect("BUG: write to String should never fail");
    writeln!(
        entrypoint,
        r#"ENTRYPOINT_DIR="$(cd "$(dirname "$0")" && pwd)""#
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
    writeln!(
        entrypoint,
        r#"PROFILE_DIR="$(dirname "$PROFILE_NEW_GENERATION")""#
    )
    .expect("BUG: write to String should never fail");
    writeln!(
        entrypoint,
        r#"if [ -z "$PROFILE_OLD_GENERATION" ]; then
  current_link="$PROFILE_DIR/current"
  if [ -L "$current_link" ]; then
    current_target="$(readlink "$current_link")"
    if [ -n "$current_target" ]; then
      case "$current_target" in
        /*) resolved_old_generation="$current_target" ;;
        *) resolved_old_generation="$PROFILE_DIR/$current_target" ;;
      esac
      if [ -e "$resolved_old_generation" ]; then
        PROFILE_OLD_GENERATION="$resolved_old_generation"
      else
        PROFILE_OLD_GENERATION=""
      fi
    else
      PROFILE_OLD_GENERATION=""
    fi
  else
    PROFILE_OLD_GENERATION=""
  fi
  export PROFILE_OLD_GENERATION
fi"#
    )
    .expect("BUG: write to String should never fail");
    writeln!(entrypoint, r#"SCRIPTS_DIR="$ENTRYPOINT_DIR/scripts""#)
        .expect("BUG: write to String should never fail");

    for script in scripts {
        writeln!(entrypoint, "\"$SCRIPTS_DIR/{script}\"")
            .expect("BUG: write to String should never fail");
    }

    entrypoint
}

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
                    "activation script filename is not valid UTF-8: {}",
                    entry.file_name().to_string_lossy()
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

    let entrypoint = write_entrypoint(&scripts);
    let entrypoint_path = activation_dir.join("entrypoint");
    std::fs::write(&entrypoint_path, &entrypoint)?;
    std::fs::set_permissions(&entrypoint_path, std::fs::Permissions::from_mode(0o755))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    struct TestEnv {
        _tempdir: tempfile::TempDir,
        old_generation: PathBuf,
        new_generation: PathBuf,
        entrypoint_path: PathBuf,
        output_path: PathBuf,
    }

    fn write_executable(path: &Path, content: &str) {
        std::fs::write(path, content).expect("BUG: should write executable");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: should set executable permissions");
    }

    fn prepare_test_env() -> TestEnv {
        let tempdir = tempfile::tempdir().expect("BUG: should create tempdir");
        let profile_dir = tempdir.path().join("profile");
        let old_generation = profile_dir.join("1-link");
        let new_generation = profile_dir.join("2-link");
        let scripts_dir = new_generation.join("core/activation/scripts");
        let entrypoint_path = new_generation.join("core/activation/entrypoint");
        let output_path = tempdir.path().join("captured-env.txt");

        std::fs::create_dir_all(&old_generation).expect("BUG: should create old generation");
        std::fs::create_dir_all(&scripts_dir).expect("BUG: should create scripts dir");
        std::os::unix::fs::symlink("1-link", profile_dir.join("current"))
            .expect("BUG: should create current symlink");

        let capture_script = format!(
            "#!/bin/sh\nset -e\nprintf 'PROFILE_NEW_GENERATION=%s\\nPROFILE_OLD_GENERATION=%s\\n' \"${{PROFILE_NEW_GENERATION-}}\" \"${{PROFILE_OLD_GENERATION-}}\" > '{}'\n",
            output_path.display()
        );
        write_executable(&scripts_dir.join("10-capture-env"), &capture_script);

        let entrypoint = super::write_entrypoint(&[String::from("10-capture-env")]);
        write_executable(&entrypoint_path, &entrypoint);

        TestEnv {
            _tempdir: tempdir,
            old_generation,
            new_generation,
            entrypoint_path,
            output_path,
        }
    }

    fn run_entrypoint(
        test_env: &TestEnv,
        new_generation: Option<&Path>,
        old_generation: Option<&Path>,
    ) {
        let mut command = Command::new(&test_env.entrypoint_path);
        command
            .env_remove("PROFILE_NEW_GENERATION")
            .env_remove("PROFILE_OLD_GENERATION");

        if let Some(path) = new_generation {
            command.env("PROFILE_NEW_GENERATION", path);
        }
        if let Some(path) = old_generation {
            command.env("PROFILE_OLD_GENERATION", path);
        }

        let status = command.status().expect("BUG: should execute entrypoint");
        assert!(status.success(), "entrypoint should exit successfully");
    }

    fn read_captured_env(output_path: &Path) -> HashMap<String, String> {
        let content = std::fs::read_to_string(output_path).expect("BUG: should read captured env");
        content
            .lines()
            .map(|line| {
                let (key, value) = line
                    .split_once('=')
                    .expect("BUG: captured env line should contain '='");
                (key.to_owned(), value.to_owned())
            })
            .collect()
    }

    #[test]
    fn entrypoint_defaults_both_generation_env_vars_when_unset() {
        let test_env = prepare_test_env();

        run_entrypoint(&test_env, None, None);

        let captured = read_captured_env(&test_env.output_path);
        assert_eq!(
            captured.get("PROFILE_NEW_GENERATION"),
            Some(&test_env.new_generation.display().to_string())
        );
        assert_eq!(
            captured.get("PROFILE_OLD_GENERATION"),
            Some(&test_env.old_generation.display().to_string())
        );
    }

    #[test]
    fn entrypoint_preserves_both_generation_env_vars_when_provided() {
        let test_env = prepare_test_env();
        let explicit_new = test_env.new_generation.join("explicit-new");
        let explicit_old = test_env.old_generation.join("explicit-old");

        run_entrypoint(&test_env, Some(&explicit_new), Some(&explicit_old));

        let captured = read_captured_env(&test_env.output_path);
        assert_eq!(
            captured.get("PROFILE_NEW_GENERATION"),
            Some(&explicit_new.display().to_string())
        );
        assert_eq!(
            captured.get("PROFILE_OLD_GENERATION"),
            Some(&explicit_old.display().to_string())
        );
    }

    #[test]
    fn entrypoint_derives_old_generation_when_only_new_is_provided() {
        let test_env = prepare_test_env();

        run_entrypoint(&test_env, Some(&test_env.new_generation), None);

        let captured = read_captured_env(&test_env.output_path);
        assert_eq!(
            captured.get("PROFILE_NEW_GENERATION"),
            Some(&test_env.new_generation.display().to_string())
        );
        assert_eq!(
            captured.get("PROFILE_OLD_GENERATION"),
            Some(&test_env.old_generation.display().to_string())
        );
    }

    #[test]
    fn entrypoint_clears_old_generation_when_current_target_does_not_exist() {
        let test_env = prepare_test_env();
        let profile_dir = test_env
            .new_generation
            .parent()
            .expect("BUG: new generation should have a parent");
        let current_link = profile_dir.join("current");
        std::fs::remove_file(&current_link).expect("BUG: should remove current symlink");
        std::os::unix::fs::symlink("missing-link", &current_link)
            .expect("BUG: should recreate current symlink");

        run_entrypoint(&test_env, Some(&test_env.new_generation), None);

        let captured = read_captured_env(&test_env.output_path);
        assert_eq!(captured.get("PROFILE_OLD_GENERATION"), Some(&String::new()));
    }

    #[test]
    fn entrypoint_derives_new_generation_when_only_old_is_provided() {
        let test_env = prepare_test_env();
        let explicit_old = test_env.old_generation.join("explicit-old");

        run_entrypoint(&test_env, None, Some(&explicit_old));

        let captured = read_captured_env(&test_env.output_path);
        assert_eq!(
            captured.get("PROFILE_NEW_GENERATION"),
            Some(&test_env.new_generation.display().to_string())
        );
        assert_eq!(
            captured.get("PROFILE_OLD_GENERATION"),
            Some(&explicit_old.display().to_string())
        );
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::{Path, PathBuf};

/// An activation script discovered from `core/activation/scripts/`.
///
/// Scripts are executed in alphanumerical order by filename.
#[derive(Debug, Clone)]
pub struct ActivationScript {
    /// Name of the script (filename without path).
    pub name: String,
    /// Full path to the executable.
    pub path: PathBuf,
}

/// Errors that can occur during activation.
#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    #[error("failed to read activation directory '{path}': {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
    #[error("activation entrypoint '{path}' failed with exit code {exit_code}")]
    EntrypointFailed { path: String, exit_code: i32 },
    #[error("activation entrypoint '{path}' was terminated by signal")]
    EntrypointSignaled { path: String },
    #[error("failed to execute activation entrypoint '{path}': {source}")]
    EntrypointExecute {
        path: String,
        source: std::io::Error,
    },
    #[error("activation entrypoint not found at '{path}'")]
    EntrypointNotFound { path: String },
}

/// Discover activation scripts from a generation directory.
///
/// Scans `gen_path/core/activation/scripts/` for executable files and
/// returns them sorted in alphanumerical order by filename.
///
/// Returns an empty vec (not an error) when the directory does not exist.
pub fn discover_activation_scripts(
    gen_path: &Path,
) -> Result<Vec<ActivationScript>, ActivationError> {
    let scripts_dir = gen_path.join("core/activation/scripts");

    if !scripts_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(&scripts_dir).map_err(|source| ActivationError::ReadDir {
        path: scripts_dir.display().to_string(),
        source,
    })?;

    let mut scripts = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|source| ActivationError::ReadDir {
            path: scripts_dir.display().to_string(),
            source,
        })?;

        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ActivationError::ReadDir {
                path: format!(
                    "non-UTF-8 filename in {}: {}",
                    scripts_dir.display(),
                    entry.file_name().display()
                ),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "filename is not valid UTF-8",
                ),
            })?
            .to_owned();

        scripts.push(ActivationScript {
            name,
            path: entry.path(),
        });
    }

    // Sort alphanumerically by name
    scripts.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(scripts)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn discover_activation_scripts_sorted() {
        let dir = tempfile::tempdir().expect("BUG: should create temp dir");
        let scripts_dir = dir.path().join("core/activation/scripts");
        std::fs::create_dir_all(&scripts_dir).expect("BUG: should create scripts dir");

        // Create scripts in non-alphabetical order
        for name in &["zzz-link-current", "50-write-boundary", "60-bmc-service"] {
            let script_path = scripts_dir.join(name);
            std::fs::write(&script_path, "#!/bin/sh\necho hello\n")
                .expect("BUG: should write script");
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("BUG: should set permissions");
        }

        let scripts =
            discover_activation_scripts(dir.path()).expect("BUG: discovery should succeed");

        assert_eq!(scripts.len(), 3);
        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["50-write-boundary", "60-bmc-service", "zzz-link-current"]
        );
    }

    #[test]
    fn discover_returns_empty_when_no_activation_dir() {
        let dir = tempfile::tempdir().expect("BUG: should create temp dir");

        let scripts = discover_activation_scripts(dir.path())
            .expect("BUG: missing dir should not be an error");

        assert!(
            scripts.is_empty(),
            "should return empty vec when core/activation/scripts/ does not exist"
        );
    }
}

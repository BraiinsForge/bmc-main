// Copyright (C) 2025  Braiins Systems s.r.o.

//! Servers-config resolution shared by init, package upgrade, and
//! server registration: the runtime `servers.json` when present, the
//! read-only shipped default otherwise. The default is served from
//! memory and never written to the runtime path.

use std::path::{Path, PathBuf};

use crate::types::ServersConfig;

/// Failure to resolve a servers configuration.
#[derive(Debug, thiserror::Error)]
pub enum LoadServersConfigError {
    #[error("failed to read servers config {path}: {source}")]
    RuntimeRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to quarantine corrupt servers config {path}: {source}")]
    Quarantine {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("no servers configuration available: neither {runtime_path} nor {default_path} exists")]
    NoConfig {
        runtime_path: String,
        default_path: String,
    },
    #[error("failed to read default servers config {path}: {source}")]
    DefaultRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse default servers config {path}: {source}")]
    DefaultParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid default servers config {path}: {reason}")]
    DefaultInvalid { path: String, reason: String },
}

/// Load the servers configuration.
///
/// The runtime file wins wholesale when present and valid; the shipped
/// default is used in memory otherwise and is never persisted to the
/// runtime path. A present-but-corrupt runtime file is quarantined to a
/// `.bcp` sibling before falling back.
///
/// # Errors
///
/// Returns [`LoadServersConfigError`] when the runtime file is
/// unreadable for reasons other than absence, when quarantining a
/// corrupt runtime file fails, when the default is present but
/// unreadable or invalid, or when no configuration exists at all.
pub fn load_servers_config(
    runtime_path: &Path,
    default_path: &Path,
) -> Result<ServersConfig, LoadServersConfigError> {
    load_servers_config_opt(runtime_path, default_path)?.ok_or_else(|| {
        LoadServersConfigError::NoConfig {
            runtime_path: runtime_path.display().to_string(),
            default_path: default_path.display().to_string(),
        }
    })
}

/// [`load_servers_config`] variant that reports the absence of any
/// configuration as `Ok(None)` instead of an error: `None` is returned
/// exactly when the runtime file and the default are both missing.
/// `register-server` uses this to decide when to bootstrap.
///
/// # Errors
///
/// Same as [`load_servers_config`], minus the no-config case.
pub fn load_servers_config_opt(
    runtime_path: &Path,
    default_path: &Path,
) -> Result<Option<ServersConfig>, LoadServersConfigError> {
    let runtime_missing = match std::fs::read_to_string(runtime_path) {
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(config) => return Ok(Some(config)),
            Err(err) => {
                let quarantined = quarantine_corrupt(runtime_path).map_err(|source| {
                    LoadServersConfigError::Quarantine {
                        path: runtime_path.display().to_string(),
                        source,
                    }
                })?;
                tracing::warn!(
                    "corrupt servers config {} ({err}); quarantined to {} and falling back to {}",
                    runtime_path.display(),
                    quarantined.display(),
                    default_path.display()
                );
                false
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(source) => {
            return Err(LoadServersConfigError::RuntimeRead {
                path: runtime_path.display().to_string(),
                source,
            });
        }
    };

    let contents = match std::fs::read_to_string(default_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && runtime_missing => {
            return Ok(None);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(LoadServersConfigError::NoConfig {
                runtime_path: runtime_path.display().to_string(),
                default_path: default_path.display().to_string(),
            });
        }
        Err(source) => {
            return Err(LoadServersConfigError::DefaultRead {
                path: default_path.display().to_string(),
                source,
            });
        }
    };
    let config: ServersConfig =
        serde_json::from_str(&contents).map_err(|source| LoadServersConfigError::DefaultParse {
            path: default_path.display().to_string(),
            source,
        })?;
    validate_default(&config, default_path)?;
    tracing::debug!("using default servers config {}", default_path.display());
    Ok(Some(config))
}

/// Reject defaults that could be mistaken for device-written state: a
/// shipped default must never carry the bootstrap marker nor duplicate
/// the factory id among its servers.
fn validate_default(
    config: &ServersConfig,
    default_path: &Path,
) -> Result<(), LoadServersConfigError> {
    if config.bootstrapped_factory {
        return Err(LoadServersConfigError::DefaultInvalid {
            path: default_path.display().to_string(),
            reason: "bootstrap marker set".to_owned(),
        });
    }
    if config.servers.iter().any(|s| s.id == config.factory.id) {
        return Err(LoadServersConfigError::DefaultInvalid {
            path: default_path.display().to_string(),
            reason: format!("factory id '{}' duplicated in servers", config.factory.id),
        });
    }
    Ok(())
}

/// Move a corrupt config aside so it can be inspected later, without
/// clobbering earlier evidence: candidates are `{path}.bcp`,
/// `{path}.bcp.1`, `{path}.bcp.2`, …, each claimed exclusively via
/// `create_new` before the rename, so concurrent recoveries can never
/// pick the same destination. Only `AlreadyExists` advances the search;
/// any other claim failure is returned. A failed rename removes the
/// empty claim (best-effort) and is returned.
fn quarantine_corrupt(path: &Path) -> std::io::Result<PathBuf> {
    let mut n = 0_usize;
    loop {
        let mut candidate = path.as_os_str().to_owned();
        candidate.push(".bcp");
        if n > 0 {
            candidate.push(format!(".{n}"));
        }
        let candidate = PathBuf::from(candidate);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => {
                if let Err(err) = std::fs::rename(path, &candidate) {
                    let _ = std::fs::remove_file(&candidate);
                    return Err(err);
                }
                return Ok(candidate);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACTORY_ONLY: &str = r#"{"factory":{"id":"forge","base_url":"https://cache.braiins.com/v1","known_public_key":"k","priority":0,"enabled":true},"servers":[]}"#;

    fn write(path: &Path, contents: &str) {
        std::fs::write(path, contents).expect("BUG: write config");
    }

    fn paths(dir: &tempfile::TempDir) -> (PathBuf, PathBuf) {
        (
            dir.path().join("servers.json"),
            dir.path().join("servers.json.default"),
        )
    }

    #[test]
    fn loads_valid_runtime_file_without_reading_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(&runtime, FACTORY_ONLY);
        write(&default, "{ not even json");

        let config = load_servers_config(&runtime, &default).expect("BUG: valid file must load");

        assert_eq!(config.factory.id, "forge");
    }

    #[test]
    fn missing_runtime_serves_default_without_creating_runtime() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(&default, FACTORY_ONLY);

        let config = load_servers_config(&runtime, &default).expect("BUG: default must serve");

        assert_eq!(config.factory.id, "forge");
        assert!(
            !runtime.exists(),
            "the default must never be persisted to the runtime path"
        );
    }

    #[test]
    fn missing_runtime_and_default_is_none_and_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);

        let opt =
            load_servers_config_opt(&runtime, &default).expect("BUG: both-missing is not an error");
        assert!(opt.is_none());

        let err = load_servers_config(&runtime, &default).expect_err("strict load must error");
        assert!(matches!(err, LoadServersConfigError::NoConfig { .. }));
    }

    #[test]
    fn corrupt_runtime_is_quarantined_and_default_served() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(&runtime, "{ this is not json");
        write(&default, FACTORY_ONLY);

        let config = load_servers_config(&runtime, &default).expect("BUG: default must recover");

        assert_eq!(config.factory.id, "forge");
        assert!(dir.path().join("servers.json.bcp").exists());
        assert!(
            !runtime.exists(),
            "quarantine moves the corrupt file; nothing recreates the runtime path"
        );
    }

    #[test]
    fn second_quarantine_uses_numbered_candidate() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(&default, FACTORY_ONLY);

        write(&runtime, "{ corrupt one");
        load_servers_config(&runtime, &default).expect("BUG: first recovery");
        write(&runtime, "{ corrupt two");
        load_servers_config(&runtime, &default).expect("BUG: second recovery");

        let first = std::fs::read_to_string(dir.path().join("servers.json.bcp"))
            .expect("BUG: first backup exists");
        let second = std::fs::read_to_string(dir.path().join("servers.json.bcp.1"))
            .expect("BUG: second backup exists");
        assert_eq!(first, "{ corrupt one");
        assert_eq!(second, "{ corrupt two");
    }

    #[test]
    fn corrupt_runtime_with_missing_default_errors_after_quarantine() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(&runtime, "{ this is not json");

        let err = load_servers_config_opt(&runtime, &default)
            .expect_err("corruption without a default must error, not bootstrap");
        assert!(matches!(err, LoadServersConfigError::NoConfig { .. }));
        assert!(dir.path().join("servers.json.bcp").exists());
    }

    #[test]
    fn malformed_default_errors() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(&default, "{ not json");

        let err = load_servers_config(&runtime, &default).expect_err("bad default must error");
        assert!(matches!(err, LoadServersConfigError::DefaultParse { .. }));
    }

    #[test]
    fn default_with_bootstrap_marker_is_invalid() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(
            &default,
            r#"{"factory":{"id":"forge","base_url":"https://cache.braiins.com/v1","known_public_key":"k","priority":0,"enabled":true},"servers":[],"bootstrapped_factory":true}"#,
        );

        let err =
            load_servers_config(&runtime, &default).expect_err("marker in default is invalid");
        assert!(matches!(err, LoadServersConfigError::DefaultInvalid { .. }));
    }

    #[test]
    fn default_duplicating_factory_id_is_invalid() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(
            &default,
            r#"{"factory":{"id":"forge","base_url":"https://cache.braiins.com/v1","known_public_key":"k","priority":0,"enabled":true},"servers":[{"id":"forge","type":"http","base_url":"https://x/v1","known_public_key":"k","priority":1,"enabled":true}]}"#,
        );

        let err = load_servers_config(&runtime, &default).expect_err("factory dup is invalid");
        assert!(matches!(err, LoadServersConfigError::DefaultInvalid { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_runtime_is_error_without_quarantine() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        std::fs::create_dir(&runtime).expect("BUG: mkdir");
        write(&default, FACTORY_ONLY);

        let err = load_servers_config(&runtime, &default).expect_err("directory read must error");
        assert!(matches!(err, LoadServersConfigError::RuntimeRead { .. }));
        assert!(!dir.path().join("servers.json.bcp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_default_is_error() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        std::fs::create_dir(&default).expect("BUG: mkdir");

        let err =
            load_servers_config(&runtime, &default).expect_err("directory default must error");
        assert!(matches!(err, LoadServersConfigError::DefaultRead { .. }));
    }

    #[test]
    fn no_config_error_names_both_paths() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);

        let err = load_servers_config(&runtime, &default).expect_err("no config must error");
        let message = err.to_string();
        assert!(message.contains(&runtime.display().to_string()));
        assert!(message.contains(&default.display().to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn quarantine_claim_failure_is_hard_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let (runtime, default) = paths(&dir);
        write(&runtime, "{ corrupt");
        write(&default, FACTORY_ONLY);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555))
            .expect("BUG: chmod");

        let result = load_servers_config(&runtime, &default);

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755))
            .expect("BUG: chmod back");
        let err = result.expect_err("claim failure must be fatal");
        assert!(matches!(err, LoadServersConfigError::Quarantine { .. }));
        assert!(
            runtime.exists(),
            "the corrupt file must stay in place for inspection"
        );
    }

    #[test]
    fn quarantine_rename_failure_removes_claim() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let missing = dir.path().join("servers.json");

        quarantine_corrupt(&missing).expect_err("renaming a missing file must error");
        assert!(
            !dir.path().join("servers.json.bcp").exists(),
            "the empty claim must be cleaned up"
        );
    }
}

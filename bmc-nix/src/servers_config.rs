// Copyright (C) 2025  Braiins Systems s.r.o.

//! Servers-config loading with recovery, shared by init and package upgrade.

use std::path::Path;

use crate::types::ServersConfig;

/// Neither the runtime `servers.json` nor its firmware-shipped
/// `servers.json.default` could be loaded.
#[derive(Debug, thiserror::Error)]
#[error("no servers configuration available: neither {path} nor its .default could be loaded")]
pub struct LoadServersConfigError {
    path: String,
}

/// Load the servers configuration, recovering from a corrupt or missing
/// runtime file.
///
/// 1. Read and parse the runtime `servers.json`.
/// 2. On failure, back the corrupt file up to `.bcp` (when it exists) and
///    load the firmware-shipped `servers.json.default`, persisting it back to
///    the runtime path (best-effort).
/// 3. If neither yields a parseable config, return an error.
///
/// # Errors
///
/// Returns [`LoadServersConfigError`] when neither the runtime file nor the
/// firmware-shipped default yields a parseable configuration.
pub fn load_servers_config(
    servers_config_path: &Path,
) -> Result<ServersConfig, LoadServersConfigError> {
    match try_load(servers_config_path) {
        Ok(config) => return Ok(config),
        Err(err) => {
            tracing::warn!("{}: {err}", servers_config_path.display());
            backup_corrupt(servers_config_path);
        }
    }

    let default_path = servers_config_path.with_extension("json.default");
    match try_load(&default_path) {
        Ok(config) => {
            if let Err(err) = persist(servers_config_path, &default_path) {
                tracing::warn!(
                    "failed to persist recovered servers config to {}: {err}",
                    servers_config_path.display()
                );
            }
            Ok(config)
        }
        Err(err) => {
            tracing::warn!("{}: {err}", default_path.display());
            Err(LoadServersConfigError {
                path: servers_config_path.display().to_string(),
            })
        }
    }
}

fn try_load(path: &Path) -> Result<ServersConfig, Box<dyn std::error::Error>> {
    let contents = std::fs::read_to_string(path)?;
    let config = serde_json::from_str(&contents)?;
    Ok(config)
}

/// Copy `src` onto `dest` atomically and durably: temporary sibling, fsync,
/// rename, then fsync the parent directory so a power cut during recovery can
/// neither truncate the runtime config nor lose the rename.
fn persist(dest: &Path, src: &Path) -> std::io::Result<()> {
    let contents = std::fs::read(src)?;
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = dest.with_extension("json.tmp");
    let mut tmp = std::fs::File::create(&tmp_path)?;
    std::io::Write::write_all(&mut tmp, &contents)?;
    tmp.sync_all()?;
    drop(tmp);
    std::fs::rename(&tmp_path, dest)?;
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Back a corrupt config file up to `{path}.bcp` so it can be inspected later.
/// Only acts when the file exists (present but unparseable, not merely absent).
fn backup_corrupt(path: &Path) {
    if !path.exists() {
        return;
    }
    let mut backup = path.as_os_str().to_owned();
    backup.push(".bcp");
    let backup = std::path::PathBuf::from(backup);
    match std::fs::rename(path, &backup) {
        Ok(()) => tracing::info!("backed up corrupt config to {}", backup.display()),
        Err(err) => tracing::warn!(
            "failed to back up {} to {}: {err}",
            path.display(),
            backup.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACTORY_ONLY: &str = r#"{"factory":{"id":"forge","base_url":"https://cache.braiins.com/v1","known_public_key":"k","priority":0,"enabled":true},"servers":[]}"#;

    fn write(path: &Path, contents: &str) {
        std::fs::write(path, contents).expect("BUG: write config");
    }

    #[test]
    fn loads_valid_runtime_file() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        write(&path, FACTORY_ONLY);

        let config = load_servers_config(&path).expect("BUG: valid file should load");

        assert_eq!(config.factory.id, "forge");
        assert!(config.servers.is_empty());
    }

    #[test]
    fn missing_file_and_missing_default_errors() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");

        load_servers_config(&path).expect_err("no config anywhere must error");
    }

    #[test]
    fn recovers_missing_file_from_default_and_persists() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        let default_path = dir.path().join("servers.json.default");
        write(&default_path, FACTORY_ONLY);

        let config = load_servers_config(&path).expect("BUG: default should recover");

        assert_eq!(config.factory.id, "forge");
        assert!(
            path.exists(),
            "recovered config must be persisted to servers.json"
        );
    }

    #[test]
    fn corrupt_file_is_backed_up_and_recovered_from_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        let default_path = dir.path().join("servers.json.default");
        write(&path, "{ this is not json");
        write(&default_path, FACTORY_ONLY);

        let config = load_servers_config(&path).expect("BUG: default should recover");

        assert_eq!(config.factory.id, "forge");
        assert!(
            dir.path().join("servers.json.bcp").exists(),
            "corrupt file must be backed up to .bcp"
        );
        assert!(path.exists(), "recovered config must be persisted");
    }

    #[test]
    fn corrupt_file_without_default_backs_up_and_errors() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("servers.json");
        write(&path, "{ this is not json");

        load_servers_config(&path).expect_err("no usable config must error");

        assert!(
            dir.path().join("servers.json.bcp").exists(),
            "corrupt file must still be backed up to .bcp"
        );
    }
}

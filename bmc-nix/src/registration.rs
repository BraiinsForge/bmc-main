// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::Path;

use crate::types::{FactoryServerEntry, ServerEntry, ServersConfig};

/// Errors returned when registering a server or substituter.
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize {path}: {source}")]
    Serialize {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Write `contents` to `path` atomically and durably: temporary sibling,
/// fsync, rename, then fsync the parent directory so a power cut can
/// neither truncate the target nor lose the rename.
fn write_persisted(path: &Path, tmp_path: &Path, contents: &str) -> Result<(), RegisterError> {
    (|| -> std::io::Result<()> {
        let mut tmp = std::fs::File::create(tmp_path)?;
        std::io::Write::write_all(&mut tmp, contents.as_bytes())?;
        tmp.sync_all()?;
        drop(tmp);
        std::fs::rename(tmp_path, path)?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })()
    .map_err(|source| RegisterError::Write {
        path: path.display().to_string(),
        source,
    })
}

/// Insert or replace a server entry in `servers.json` by `id`.
///
/// Reads the existing [`ServersConfig`], drops any entry sharing the new
/// entry's `id`, appends the new entry, and writes the result back
/// atomically via a temporary sibling file plus `rename`. A missing
/// config is bootstrapped with the registered server doubling as the
/// mandatory `factory` entry, so a dev-provisioned device (`deck init`
/// writes no registry) becomes usable without manual seeding.
///
/// # Errors
///
/// Returns [`RegisterError`] if the config cannot be read, parsed, or
/// written back.
pub fn register_server(
    servers_config_path: &Path,
    entry: ServerEntry,
) -> Result<(), RegisterError> {
    let path_str = servers_config_path.display().to_string();

    let mut config: ServersConfig = match std::fs::read_to_string(servers_config_path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|source| RegisterError::Parse {
            path: path_str.clone(),
            source,
        })?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => ServersConfig {
            factory: FactoryServerEntry {
                id: entry.id.clone(),
                base_url: entry.base_url.clone(),
                known_public_key: entry.known_public_key.clone(),
                priority: entry.priority,
                enabled: entry.enabled,
            },
            servers: Vec::new(),
        },
        Err(source) => {
            return Err(RegisterError::Read {
                path: path_str,
                source,
            });
        }
    };

    config.servers.retain(|s| s.id != entry.id);
    config.servers.push(entry);

    let serialized =
        serde_json::to_string_pretty(&config).map_err(|source| RegisterError::Serialize {
            path: path_str,
            source,
        })?;

    let tmp_path = servers_config_path.with_extension("json.tmp");
    write_persisted(servers_config_path, &tmp_path, &serialized)
}

/// Register a substituter and its trusted public key in `nix.conf`.
///
/// Merges `<url>` into the `extra-substituters` line and `<public_key>`
/// into the `extra-trusted-public-keys` line, appending each token only
/// when it is not already present so registering a second server keeps
/// the first. Idempotent: a token already present leaves the file
/// unchanged and skips the rewrite. A missing `nix.conf` is treated as
/// empty and created. The write is atomic via a temporary sibling plus
/// `rename`.
///
/// # Errors
///
/// Returns [`RegisterError`] if the config cannot be read or written.
pub fn register_substituter(
    nix_conf_path: &Path,
    url: &str,
    public_key: &str,
) -> Result<(), RegisterError> {
    let path_str = nix_conf_path.display().to_string();

    let existing = match std::fs::read_to_string(nix_conf_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(RegisterError::Read {
                path: path_str,
                source,
            });
        }
    };

    let mut lines: Vec<String> = existing.lines().map(str::to_owned).collect();
    for (key, value) in [
        ("extra-substituters", url),
        ("extra-trusted-public-keys", public_key),
    ] {
        match lines.iter().position(|l| config_key(l) == Some(key)) {
            Some(pos) => {
                let mut tokens: Vec<&str> = config_value(&lines[pos]).split_whitespace().collect();
                if !tokens.contains(&value) {
                    tokens.push(value);
                }
                lines[pos] = format!("{key} = {}", tokens.join(" "));
            }
            None => lines.push(format!("{key} = {value}")),
        }
    }

    let mut updated = lines.join("\n");
    updated.push('\n');

    if updated == existing {
        return Ok(());
    }

    let tmp_path = nix_conf_path.with_extension("conf.tmp");
    write_persisted(nix_conf_path, &tmp_path, &updated)
}

/// Return the trimmed key of a `key = value` line, if it has one.
fn config_key(line: &str) -> Option<&str> {
    line.split_once('=').map(|(key, _)| key.trim())
}

/// Return the trimmed value of a `key = value` line, or empty if it has none.
fn config_value(line: &str) -> &str {
    line.split_once('=').map_or("", |(_, value)| value.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(id: &str, base_url: &str) -> ServerEntry {
        ServerEntry {
            id: id.to_owned(),
            server_type: "cache".to_owned(),
            base_url: base_url.to_owned(),
            known_public_key: "cache.example.com:AAAA".to_owned(),
            priority: 10,
            enabled: true,
            required: true,
        }
    }

    fn write_base_config(path: &Path) {
        let json = r#"{
  "factory": {
    "id": "braiins",
    "base_url": "https://cache.braiins.com/v1",
    "known_public_key": "cache.braiins.com:placeholder",
    "priority": 1,
    "enabled": true
  },
  "servers": []
}"#;
        std::fs::write(path, json).expect("BUG: write base servers.json");
    }

    fn read_config(path: &Path) -> ServersConfig {
        let raw = std::fs::read_to_string(path).expect("BUG: read servers.json");
        serde_json::from_str(&raw).expect("BUG: parse servers.json")
    }

    #[test]
    fn register_server_inserts_then_replaces_by_id() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        write_base_config(&path);

        register_server(&path, sample_entry("dev", "https://dev.example.com/v1"))
            .expect("BUG: first register");
        let config = read_config(&path);
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].id, "dev");
        assert_eq!(config.servers[0].base_url, "https://dev.example.com/v1");
        assert!(config.servers[0].enabled);

        register_server(&path, sample_entry("dev", "https://dev.example.com/v2"))
            .expect("BUG: second register");
        let config = read_config(&path);
        assert_eq!(
            config.servers.len(),
            1,
            "same id must replace, not duplicate"
        );
        assert_eq!(config.servers[0].base_url, "https://dev.example.com/v2");
    }

    #[test]
    fn register_server_bootstraps_missing_config_with_entry_as_factory() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");

        register_server(&path, sample_entry("dev", "https://dev.example.com/v1"))
            .expect("BUG: bootstrap register");

        let config = read_config(&path);
        assert_eq!(config.factory.id, "dev");
        assert_eq!(config.factory.base_url, "https://dev.example.com/v1");
        assert_eq!(config.factory.known_public_key, "cache.example.com:AAAA");
        assert!(config.factory.enabled);
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].id, "dev");
    }

    #[test]
    fn register_substituter_is_idempotent() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("nix.conf");

        let url = "https://cache.example.com";
        let key = "cache.example.com:AAAA";

        register_substituter(&path, url, key).expect("BUG: first register");
        register_substituter(&path, url, key).expect("BUG: second register");

        let contents = std::fs::read_to_string(&path).expect("BUG: read nix.conf");
        let substituter_hits = contents
            .lines()
            .filter(|l| *l == format!("extra-substituters = {url}"))
            .count();
        let key_hits = contents
            .lines()
            .filter(|l| *l == format!("extra-trusted-public-keys = {key}"))
            .count();
        assert_eq!(substituter_hits, 1, "substituter line must appear once");
        assert_eq!(key_hits, 1, "trusted key line must appear once");
    }

    #[test]
    fn register_substituter_merges_multiple_servers() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("nix.conf");
        register_substituter(&path, "https://a.example.com", "a:KEYA").expect("BUG: first");
        register_substituter(&path, "https://b.example.com", "b:KEYB").expect("BUG: second");
        let contents = std::fs::read_to_string(&path).expect("BUG: read");
        assert!(
            contents
                .lines()
                .any(|l| l == "extra-substituters = https://a.example.com https://b.example.com")
        );
        assert!(
            contents
                .lines()
                .any(|l| l == "extra-trusted-public-keys = a:KEYA b:KEYB")
        );
    }

    #[test]
    fn register_substituter_accumulates_rotated_values() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("nix.conf");
        std::fs::write(
            &path,
            "substituters = https://cache.braiins.com\nextra-experimental-features = nix-command flakes\n",
        )
        .expect("BUG: write base nix.conf");

        register_substituter(&path, "https://old.example.com", "dev-upgrade:OLD")
            .expect("BUG: first register");
        register_substituter(&path, "https://new.example.com", "dev-upgrade:NEW")
            .expect("BUG: rotated register");

        let contents = std::fs::read_to_string(&path).expect("BUG: read nix.conf");

        let substituter_lines: Vec<&str> = contents
            .lines()
            .filter(|l| config_key(l) == Some("extra-substituters"))
            .collect();
        let key_lines: Vec<&str> = contents
            .lines()
            .filter(|l| config_key(l) == Some("extra-trusted-public-keys"))
            .collect();
        assert_eq!(
            substituter_lines,
            vec!["extra-substituters = https://old.example.com https://new.example.com"],
            "rotated url accumulates alongside the old one (removal is out of scope)"
        );
        assert_eq!(
            key_lines,
            vec!["extra-trusted-public-keys = dev-upgrade:OLD dev-upgrade:NEW"],
            "rotated key accumulates alongside the old one (removal is out of scope)"
        );
        assert!(
            contents
                .lines()
                .any(|l| l == "substituters = https://cache.braiins.com"),
            "pre-existing system settings must be preserved"
        );
    }
}

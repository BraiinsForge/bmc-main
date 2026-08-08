// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::path::{Path, PathBuf};

use crate::types::{FactoryServerEntry, ServerEntry, ServersConfig};

/// What a registration does to the server entries it is not registering.
///
/// Resolution ranks candidates by version before it consults priority,
/// so a lower priority cannot keep a rig authoritative
/// against a public server that publishes something newer.
/// Disabling the others is the only way to let one server decide alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherServers {
    /// Leave every other entry as it stands.
    Keep,
    /// Disable every other entry, leaving the factory entry alone —
    /// it anchors init trust and takes no part in upgrade resolution.
    Disable,
}

impl From<bool> for OtherServers {
    fn from(exclusive: bool) -> Self {
        if exclusive { Self::Disable } else { Self::Keep }
    }
}

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
    #[error(transparent)]
    Load(#[from] crate::servers_config::LoadServersConfigError),
    #[error("server id '{id}' collides with the factory server id; pick a different --id")]
    FactoryIdCollision { id: String },
    #[error(
        "registering '{id}' seeds or updates the factory entry; --factory-base-url is required"
    )]
    FactoryBaseUrlRequired { id: String },
}

/// Write `contents` to `path` atomically and durably: temporary sibling,
/// fsync, rename, then fsync the parent directory so a power cut can
/// neither truncate the target nor lose the rename.
fn write_persisted(path: &Path, tmp_path: &Path, contents: &str) -> Result<(), RegisterError> {
    (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut tmp = std::fs::File::create(tmp_path)?;
        std::io::Write::write_all(&mut tmp, contents.as_bytes())?;
        tmp.sync_all()?;
        drop(tmp);
        std::fs::rename(tmp_path, path)?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::fs_sync::fsync_dir(parent)?;
        }
        Ok(())
    })()
    .map_err(|source| RegisterError::Write {
        path: path.display().to_string(),
        source,
    })
}

/// A validated servers-config update, ready to be written.
///
/// Holds the serialized config and destination returned by either preparation helper.
/// [`Self::persist`] performs the atomic write without re-reading the source config.
#[derive(Debug)]
pub struct PreparedRegistration {
    path: PathBuf,
    serialized: String,
}

impl PreparedRegistration {
    /// Write the prepared config atomically via a temporary sibling
    /// plus `rename`.
    ///
    /// # Errors
    ///
    /// Returns [`RegisterError`] if the write fails.
    pub fn persist(&self) -> Result<(), RegisterError> {
        let tmp_path = self.path.with_extension("json.tmp");
        write_persisted(&self.path, &tmp_path, &self.serialized)
    }
}

/// Load the servers config, insert or replace `entry` by `id`, and
/// serialize the result without writing it.
///
/// The config comes from the runtime file when present, from the
/// shipped default when only the runtime file is missing (preserving
/// the shipped factory entry), and — when neither exists — from an
/// entry-as-factory bootstrap marked with `bootstrapped_factory`, so a
/// dev-provisioned device becomes usable without manual seeding.
/// Registering an id equal to the loaded factory id is rejected unless
/// the config is marker-bootstrapped, in which case the factory and the
/// matching server entry are updated in sync. `factory_base_url`
/// supplies the factory entry's base URL; it is required whenever the
/// registration seeds or updates the factory entry — the no-config
/// bootstrap and the bootstrapped same-id resync — and ignored
/// otherwise. `other_servers` decides whether the entries left behind
/// keep resolving; see [`OtherServers`].
///
/// # Errors
///
/// Returns [`RegisterError`] if the config cannot be loaded, if the id
/// collides with a non-bootstrapped factory, if the factory base URL is
/// required but missing, or if serialization fails.
pub fn prepare_registration(
    runtime_path: &Path,
    default_path: &Path,
    entry: ServerEntry,
    factory_base_url: Option<&str>,
    other_servers: OtherServers,
) -> Result<PreparedRegistration, RegisterError> {
    let require_factory_base_url = || {
        factory_base_url.ok_or_else(|| RegisterError::FactoryBaseUrlRequired {
            id: entry.id.clone(),
        })
    };
    let mut config =
        match crate::servers_config::load_servers_config_opt(runtime_path, default_path)? {
            Some(config) => config,
            None => ServersConfig {
                factory: FactoryServerEntry {
                    id: entry.id.clone(),
                    base_url: require_factory_base_url()?.to_owned(),
                    known_public_key: entry.known_public_key.clone(),
                    priority: entry.priority,
                    enabled: entry.enabled,
                },
                servers: Vec::new(),
                bootstrapped_factory: true,
            },
        };

    if entry.id == config.factory.id {
        if !config.bootstrapped_factory {
            return Err(RegisterError::FactoryIdCollision { id: entry.id });
        }
        require_factory_base_url()?.clone_into(&mut config.factory.base_url);
        config
            .factory
            .known_public_key
            .clone_from(&entry.known_public_key);
        config.factory.priority = entry.priority;
        config.factory.enabled = entry.enabled;
    }

    config.servers.retain(|s| s.id != entry.id);
    if other_servers == OtherServers::Disable {
        for server in &mut config.servers {
            server.enabled = false;
        }
    }
    config.servers.push(entry);

    let serialized =
        serde_json::to_string_pretty(&config).map_err(|source| RegisterError::Serialize {
            path: runtime_path.display().to_string(),
            source,
        })?;

    Ok(PreparedRegistration {
        path: runtime_path.to_path_buf(),
        serialized,
    })
}

/// Load the servers config and empty its package-server list.
///
/// A default-only config is prepared for the runtime path.
/// This prevents later loads from falling back to the intact default.
/// The factory entry and bootstrap marker are preserved.
///
/// # Errors
///
/// Returns [`RegisterError`] if the config cannot be loaded or serialized.
pub fn prepare_clear_servers(
    runtime_path: &Path,
    default_path: &Path,
) -> Result<Option<PreparedRegistration>, RegisterError> {
    let Some(mut config) =
        crate::servers_config::load_servers_config_opt(runtime_path, default_path)?
    else {
        return Ok(None);
    };
    config.servers.clear();

    let serialized =
        serde_json::to_string_pretty(&config).map_err(|source| RegisterError::Serialize {
            path: runtime_path.display().to_string(),
            source,
        })?;

    Ok(Some(PreparedRegistration {
        path: runtime_path.to_path_buf(),
        serialized,
    }))
}

/// Register a substituter and its trusted public key in `nix.conf`.
///
/// Merges `<url>` into the `extra-substituters` line and `<public_key>`
/// into the `extra-trusted-public-keys` line, appending each token only
/// when it is not already present so registering a second server keeps
/// the first. A key whose name (the token before the `:`) matches an
/// existing one replaces it: nix trusts only the first key per name, so
/// an appended rotated key would be silently ignored. Idempotent: a
/// token already present leaves the file unchanged and skips the
/// rewrite. A missing `nix.conf` is treated as empty and created. The
/// write is atomic via a temporary sibling plus `rename`.
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
                if key == "extra-trusted-public-keys" {
                    tokens.retain(|t| *t == value || public_key_name(t) != public_key_name(value));
                }
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

/// Return the name part of a `name:base64` public-key token.
fn public_key_name(token: &str) -> &str {
    token.split_once(':').map_or(token, |(name, _)| name)
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
    use crate::types::ServerSource;

    const RUNTIME_WITH_SERVERS: &str = r#"{
  "factory": {
    "id": "factory",
    "base_url": "https://factory.example",
    "known_public_key": "factory:key",
    "priority": 10,
    "enabled": true
  },
  "servers": [{
    "id": "forge",
    "index_url": "https://forge.example/index.json",
    "known_public_key": "forge:key",
    "priority": 50,
    "enabled": true,
    "required": true
  }]
}"#;

    fn sample_entry(id: &str, index_url: &str) -> ServerEntry {
        ServerEntry {
            id: id.to_owned(),
            source: ServerSource::Index {
                index_url: index_url.to_owned(),
            },
            known_public_key: "cache.example.com:AAAA".to_owned(),
            priority: 10,
            enabled: true,
            required: true,
        }
    }

    fn feed_entry(id: &str, feed_url: &str) -> ServerEntry {
        ServerEntry {
            source: ServerSource::Feed {
                feed_url: feed_url.to_owned(),
            },
            ..sample_entry(id, "https://unused.example.com/i.json")
        }
    }

    fn entry_index_url(entry: &ServerEntry) -> &str {
        match &entry.source {
            ServerSource::Index { index_url } => index_url,
            ServerSource::Feed { feed_url } => {
                panic!("BUG: expected an index-linked entry, got feed {feed_url}")
            }
        }
    }

    fn write_base_config(path: &Path) {
        let json = r#"{
  "factory": {
    "id": "forge",
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

    fn register(path: &Path, entry: ServerEntry) -> Result<(), RegisterError> {
        register_with_factory(path, entry, None)
    }

    fn register_with_factory(
        path: &Path,
        entry: ServerEntry,
        factory_base_url: Option<&str>,
    ) -> Result<(), RegisterError> {
        register_entry(path, entry, factory_base_url, OtherServers::Keep)
    }

    fn register_exclusively(path: &Path, entry: ServerEntry) -> Result<(), RegisterError> {
        register_entry(path, entry, None, OtherServers::Disable)
    }

    fn register_entry(
        path: &Path,
        entry: ServerEntry,
        factory_base_url: Option<&str>,
        other_servers: OtherServers,
    ) -> Result<(), RegisterError> {
        let mut default = path.as_os_str().to_owned();
        default.push(".default");
        prepare_registration(
            path,
            Path::new(&default),
            entry,
            factory_base_url,
            other_servers,
        )?
        .persist()
    }

    #[test]
    fn exclusive_registration_disables_the_servers_it_must_outrank() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        write_base_config(&path);

        register(
            &path,
            sample_entry("forge-feed", "https://forge.example.com/v1"),
        )
        .expect("BUG: register the public server");
        register_exclusively(&path, sample_entry("rig", "http://10.0.0.1:8083/v1"))
            .expect("BUG: register the rig exclusively");

        let config = read_config(&path);
        let enabled: Vec<&str> = config
            .servers
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(
            enabled,
            vec!["rig"],
            "an exclusive registration must leave only its own server resolving, \
             since resolution ranks version above priority"
        );
    }

    #[test]
    fn exclusive_registration_spares_the_factory_entry() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        write_base_config(&path);

        register_exclusively(&path, sample_entry("rig", "http://10.0.0.1:8083/v1"))
            .expect("BUG: register the rig exclusively");

        let config = read_config(&path);
        assert!(
            config.factory.enabled,
            "the factory entry anchors init trust and takes no part in upgrade \
             resolution, so exclusivity must not disable it"
        );
    }

    #[test]
    fn non_exclusive_registration_leaves_other_servers_enabled() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        write_base_config(&path);

        register(
            &path,
            sample_entry("forge-feed", "https://forge.example.com/v1"),
        )
        .expect("BUG: register the public server");
        register(&path, sample_entry("rig", "http://10.0.0.1:8083/v1"))
            .expect("BUG: register the rig");

        let config = read_config(&path);
        assert!(
            config.servers.iter().all(|s| s.enabled),
            "a plain registration must not disturb the servers already registered"
        );
    }

    #[test]
    fn clear_servers_empties_the_list_and_keeps_the_factory() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let runtime = dir.path().join("servers.json");
        let default = dir.path().join("servers.json.default");
        std::fs::write(&runtime, RUNTIME_WITH_SERVERS).expect("BUG: write runtime");

        let prepared = prepare_clear_servers(&runtime, &default)
            .expect("BUG: prepare failed")
            .expect("BUG: a present config must be clearable");
        prepared.persist().expect("BUG: persist failed");

        let written = read_config(&runtime);
        assert!(written.servers.is_empty());
        assert_eq!(written.factory.id, "factory");
        assert_eq!(written.factory.base_url, "https://factory.example");
        assert_eq!(written.factory.known_public_key, "factory:key");
        assert_eq!(written.factory.priority, 10);
        assert!(written.factory.enabled);
    }

    #[test]
    fn clear_servers_materializes_the_runtime_file_from_the_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let runtime = dir.path().join("servers.json");
        let default = dir.path().join("servers.json.default");
        std::fs::write(&default, RUNTIME_WITH_SERVERS).expect("BUG: write default");

        let prepared = prepare_clear_servers(&runtime, &default)
            .expect("BUG: prepare failed")
            .expect("BUG: a default-only config must still be clearable");
        prepared.persist().expect("BUG: persist failed");

        assert!(read_config(&runtime).servers.is_empty());
        assert_eq!(
            read_config(&default).servers.len(),
            1,
            "the shipped default stays pristine"
        );
    }

    #[test]
    fn clear_servers_without_any_config_is_a_noop() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let prepared = prepare_clear_servers(
            &dir.path().join("servers.json"),
            &dir.path().join("servers.json.default"),
        )
        .expect("BUG: prepare failed");

        assert!(prepared.is_none());
    }

    #[test]
    fn clear_servers_keeps_the_bootstrapped_factory_marker() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let runtime = dir.path().join("servers.json");
        let default = dir.path().join("servers.json.default");
        let bootstrapped = RUNTIME_WITH_SERVERS.replacen(
            "{\n  \"factory\"",
            "{\n  \"bootstrapped_factory\": true,\n  \"factory\"",
            1,
        );
        std::fs::write(&runtime, bootstrapped).expect("BUG: write runtime");

        prepare_clear_servers(&runtime, &default)
            .expect("BUG: prepare failed")
            .expect("BUG: a present config must be clearable")
            .persist()
            .expect("BUG: persist failed");

        assert!(
            read_config(&runtime).bootstrapped_factory,
            "the bootstrap marker must survive a clear"
        );
    }

    #[test]
    fn clear_servers_quarantines_a_corrupt_runtime_and_clears_the_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let runtime = dir.path().join("servers.json");
        let default = dir.path().join("servers.json.default");
        std::fs::write(&runtime, "not json").expect("BUG: write runtime");
        std::fs::write(&default, RUNTIME_WITH_SERVERS).expect("BUG: write default");

        prepare_clear_servers(&runtime, &default)
            .expect("BUG: a corrupt runtime file must quarantine, not fail the clear")
            .expect("BUG: the default fallback must be clearable")
            .persist()
            .expect("BUG: persist failed");

        assert!(
            dir.path().join("servers.json.bcp").exists(),
            "the corrupt runtime file must be quarantined beside the new one"
        );
        assert!(read_config(&runtime).servers.is_empty());
    }

    #[test]
    fn register_inserts_then_replaces_by_id() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        write_base_config(&path);

        register(&path, sample_entry("dev", "https://dev.example.com/v1"))
            .expect("BUG: first register");
        let config = read_config(&path);
        assert_eq!(config.servers.len(), 1);
        assert_eq!(
            entry_index_url(&config.servers[0]),
            "https://dev.example.com/v1"
        );

        register(&path, sample_entry("dev", "https://dev.example.com/v2"))
            .expect("BUG: second register");
        let config = read_config(&path);
        assert_eq!(
            config.servers.len(),
            1,
            "same id must replace, not duplicate"
        );
        assert_eq!(
            entry_index_url(&config.servers[0]),
            "https://dev.example.com/v2"
        );
    }

    #[test]
    fn register_seeds_missing_runtime_from_default() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        write_base_config(&tmp.path().join("servers.json.default"));

        register(&path, sample_entry("dev", "https://dev.example.com/v1"))
            .expect("BUG: seeded register");

        let config = read_config(&path);
        assert_eq!(
            config.factory.id, "forge",
            "shipped factory entry must be preserved"
        );
        assert!(!config.bootstrapped_factory);
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].id, "dev");
    }

    #[test]
    fn register_bootstraps_when_runtime_and_default_missing() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");

        register_with_factory(
            &path,
            sample_entry("dev", "https://dev.example.com/v1/i.json"),
            Some("https://dev.example.com/factory"),
        )
        .expect("BUG: bootstrap register");

        let config = read_config(&path);
        assert_eq!(config.factory.id, "dev");
        assert_eq!(config.factory.base_url, "https://dev.example.com/factory");
        assert!(
            config.bootstrapped_factory,
            "bootstrap must record its provenance"
        );
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].id, "dev");
    }

    #[test]
    fn register_bootstrap_without_factory_base_url_fails() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");

        let err = register(
            &path,
            sample_entry("dev", "https://dev.example.com/v1/i.json"),
        )
        .expect_err("bootstrap without --factory-base-url must fail");
        assert!(matches!(err, RegisterError::FactoryBaseUrlRequired { .. }));
        assert!(!path.exists(), "a failed bootstrap must not write a config");
    }

    #[test]
    fn reregister_after_bootstrap_updates_factory_and_server_in_sync() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");

        register_with_factory(
            &path,
            sample_entry("dev", "https://dev.example.com/v1/i.json"),
            Some("https://dev.example.com/factory1"),
        )
        .expect("BUG: bootstrap register");
        register_with_factory(
            &path,
            sample_entry("dev", "https://dev.example.com/v2/i.json"),
            Some("https://dev.example.com/factory2"),
        )
        .expect("BUG: re-register on a bootstrapped config");

        let config = read_config(&path);
        assert_eq!(
            config.factory.base_url, "https://dev.example.com/factory2",
            "the factory base URL must track the second --factory-base-url"
        );
        assert!(config.bootstrapped_factory);
        assert_eq!(config.servers.len(), 1);
        assert_eq!(
            entry_index_url(&config.servers[0]),
            "https://dev.example.com/v2/i.json"
        );
    }

    #[test]
    fn reregister_after_bootstrap_without_factory_base_url_fails() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");

        register_with_factory(
            &path,
            sample_entry("dev", "https://dev.example.com/v1/i.json"),
            Some("https://dev.example.com/factory1"),
        )
        .expect("BUG: bootstrap register");
        let err = register(
            &path,
            sample_entry("dev", "https://dev.example.com/v2/i.json"),
        )
        .expect_err("a bootstrapped same-id re-registration requires --factory-base-url");
        assert!(matches!(err, RegisterError::FactoryBaseUrlRequired { .. }));
    }

    #[test]
    fn register_persists_and_reloads_both_source_variants() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        write_base_config(&path);

        register(
            &path,
            sample_entry("direct", "https://direct.example.com/i.json"),
        )
        .expect("BUG: register index-linked entry");
        register(
            &path,
            feed_entry("feeder", "https://feeder.example.com/f.json"),
        )
        .expect("BUG: register feed-linked entry");

        let config = read_config(&path);
        assert_eq!(config.servers.len(), 2);
        assert!(matches!(
            &config.servers[0].source,
            ServerSource::Index { index_url } if index_url == "https://direct.example.com/i.json"
        ));
        assert!(matches!(
            &config.servers[1].source,
            ServerSource::Feed { feed_url } if feed_url == "https://feeder.example.com/f.json"
        ));
    }

    #[test]
    fn register_rejects_factory_id_collision_without_marker() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        write_base_config(&path);

        let err = register(&path, sample_entry("forge", "https://evil.example.com/v1"))
            .expect_err("factory id collision must be rejected");
        assert!(matches!(err, RegisterError::FactoryIdCollision { .. }));
    }

    #[test]
    fn register_rejects_collision_on_unmarked_bootstrap_lookalike() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        std::fs::write(
            &path,
            r#"{"factory":{"id":"dev","base_url":"https://dev.example.com/v1","known_public_key":"k","priority":50,"enabled":true},"servers":[{"id":"dev","index_url":"https://dev.example.com/v1/i.json","known_public_key":"k","priority":50,"enabled":true}]}"#,
        )
        .expect("BUG: write lookalike config");

        let err = register(&path, sample_entry("dev", "https://dev.example.com/v2"))
            .expect_err("unmarked bootstrap lookalike must reject the collision");
        assert!(matches!(err, RegisterError::FactoryIdCollision { .. }));
    }

    #[test]
    fn register_errors_on_corrupt_runtime_without_default() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("servers.json");
        std::fs::write(&path, "{ corrupt").expect("BUG: write corrupt config");

        register(&path, sample_entry("dev", "https://dev.example.com/v1"))
            .expect_err("corruption without a default must not bootstrap");
        assert!(tmp.path().join("servers.json.bcp").exists());
        assert!(!path.exists(), "prepare must not recreate the runtime file");
    }

    #[test]
    fn register_creates_missing_parent_directory() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let path = tmp.path().join("nix-upgrade").join("servers.json");

        register_with_factory(
            &path,
            sample_entry("dev", "https://dev.example.com/v1/i.json"),
            Some("https://dev.example.com/factory"),
        )
        .expect("BUG: register into a missing directory");

        assert!(path.exists());
        assert_eq!(read_config(&path).factory.id, "dev");
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
    fn register_substituter_replaces_a_rotated_key_of_the_same_name() {
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
            vec!["extra-trusted-public-keys = dev-upgrade:NEW"],
            "nix trusts only the first key per name, so the rotated key must replace the old one"
        );
        assert!(
            contents
                .lines()
                .any(|l| l == "substituters = https://cache.braiins.com"),
            "pre-existing system settings must be preserved"
        );
    }
}

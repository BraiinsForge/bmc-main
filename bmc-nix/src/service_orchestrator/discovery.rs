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

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::service_orchestrator::config::ServiceConfig;

pub type ServicePriorityMap = HashMap<String, u16>;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("failed to read directory '{path}': {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to read file '{path}': {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse service config '{path}': {source}")]
    ParseConfig {
        path: String,
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredService {
    pub name: String,
    pub init_path: PathBuf,
    pub init_contents: Vec<u8>,
    pub config: ServiceConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationServices {
    pub root: PathBuf,
    pub init_services: BTreeMap<String, DiscoveredService>,
    pub start_order: ServicePriorityMap,
    pub stop_order: ServicePriorityMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceChangeKind {
    New,
    Removed,
    Upgraded,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceChange {
    pub name: String,
    pub kind: ServiceChangeKind,
    pub old: Option<DiscoveredService>,
    pub new: Option<DiscoveredService>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServiceChanges {
    pub new: Vec<ServiceChange>,
    pub removed: Vec<ServiceChange>,
    pub upgraded: Vec<ServiceChange>,
    pub unchanged: Vec<ServiceChange>,
}

#[must_use]
pub fn parse_rcd_link_name(name: &str) -> Option<(String, u16)> {
    let prefix = name.chars().next()?;
    if !matches!(prefix, 'S' | 'K') {
        return None;
    }

    let rest = name.get(1..)?;
    let digits_len = rest.chars().take_while(char::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }

    let (priority, service) = rest.split_at(digits_len);
    if service.is_empty() {
        return None;
    }

    Some((service.to_owned(), priority.parse().ok()?))
}

fn read_dir_names(dir: &Path) -> Result<Vec<String>, DiscoveryError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|source| DiscoveryError::ReadDir {
        path: dir.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| DiscoveryError::ReadDir {
            path: dir.display().to_string(),
            source,
        })?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| DiscoveryError::ReadDir {
                path: dir.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "directory entry name is not valid UTF-8",
                ),
            })?;
        names.push(name);
    }

    Ok(names)
}

fn read_init_service(
    root: &Path,
    init_dir: &Path,
    service: &str,
) -> Result<DiscoveredService, DiscoveryError> {
    let init_path = init_dir.join(service);
    let init_contents = std::fs::read(&init_path).map_err(|source| DiscoveryError::ReadFile {
        path: init_path.display().to_string(),
        source,
    })?;
    let config = load_service_config(root, service)?;

    Ok(DiscoveredService {
        name: service.to_owned(),
        init_path,
        init_contents,
        config,
    })
}

pub fn load_service_config(root: &Path, service: &str) -> Result<ServiceConfig, DiscoveryError> {
    let config_path = root.join("etc/init.d.conf").join(format!("{service}.json"));
    if !config_path.exists() {
        return Ok(ServiceConfig::default());
    }

    let contents =
        std::fs::read_to_string(&config_path).map_err(|source| DiscoveryError::ReadFile {
            path: config_path.display().to_string(),
            source,
        })?;
    serde_json::from_str(&contents).map_err(|source| DiscoveryError::ParseConfig {
        path: config_path.display().to_string(),
        source,
    })
}

pub fn discover_generation(root: &Path) -> Result<GenerationServices, DiscoveryError> {
    // Empty root signals "no previous generation" (e.g., first activation).
    // Return empty services so all new-generation services are classified as New.
    if root.as_os_str().is_empty() {
        return Ok(GenerationServices {
            root: root.to_path_buf(),
            init_services: BTreeMap::new(),
            start_order: HashMap::new(),
            stop_order: HashMap::new(),
        });
    }

    let init_dir = root.join("etc/init.d");
    let init_services = if init_dir.exists() {
        let mut entries = BTreeMap::new();
        for name in read_dir_names(&init_dir)? {
            let service = read_init_service(root, &init_dir, &name)?;
            entries.insert(name, service);
        }
        entries
    } else {
        BTreeMap::new()
    };

    let rc_dir = root.join("etc/rc.d");
    let mut start_order = HashMap::new();
    let mut stop_order = HashMap::new();
    for name in read_dir_names(&rc_dir)? {
        if let Some((service, priority)) = parse_rcd_link_name(&name) {
            match name.chars().next() {
                Some('S') => {
                    start_order.insert(service, priority);
                }
                Some('K') => {
                    stop_order.insert(service, priority);
                }
                _ => {}
            }
        }
    }

    Ok(GenerationServices {
        root: root.to_path_buf(),
        init_services,
        start_order,
        stop_order,
    })
}

fn order_priority(order: &ServicePriorityMap, service: &str) -> Option<u16> {
    order.get(service).copied()
}

fn sort_changes(changes: &mut [ServiceChange], order: &ServicePriorityMap) {
    changes.sort_by(|left, right| {
        match (
            order_priority(order, &left.name),
            order_priority(order, &right.name),
        ) {
            (Some(left_priority), Some(right_priority)) => left_priority
                .cmp(&right_priority)
                .then_with(|| left.name.cmp(&right.name)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left.name.cmp(&right.name),
        }
    });
}

#[must_use]
pub fn compare_generation_services(
    old: &GenerationServices,
    new: &GenerationServices,
) -> ServiceChanges {
    let mut changes = ServiceChanges::default();
    let mut names = std::collections::BTreeSet::new();
    names.extend(old.init_services.keys().cloned());
    names.extend(new.init_services.keys().cloned());

    for name in names {
        match (old.init_services.get(&name), new.init_services.get(&name)) {
            (Some(old_service), Some(new_service))
                if old_service.init_contents == new_service.init_contents =>
            {
                changes.unchanged.push(ServiceChange {
                    name,
                    kind: ServiceChangeKind::Unchanged,
                    old: Some(old_service.clone()),
                    new: Some(new_service.clone()),
                });
            }
            (Some(old_service), Some(new_service)) => {
                changes.upgraded.push(ServiceChange {
                    name,
                    kind: ServiceChangeKind::Upgraded,
                    old: Some(old_service.clone()),
                    new: Some(new_service.clone()),
                });
            }
            (Some(old_service), None) => {
                changes.removed.push(ServiceChange {
                    name,
                    kind: ServiceChangeKind::Removed,
                    old: Some(old_service.clone()),
                    new: None,
                });
            }
            (None, Some(new_service)) => {
                changes.new.push(ServiceChange {
                    name,
                    kind: ServiceChangeKind::New,
                    old: None,
                    new: Some(new_service.clone()),
                });
            }
            (None, None) => unreachable!("BUG: service name collected from union should exist"),
        }
    }

    sort_changes(&mut changes.removed, &old.stop_order);
    sort_changes(&mut changes.upgraded, &new.start_order);
    sort_changes(&mut changes.new, &new.start_order);
    sort_changes(&mut changes.unchanged, &new.start_order);

    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("BUG: should create parent directories");
        }
        std::fs::write(path, contents).expect("BUG: should write file");
    }

    #[test]
    fn sorts_start_actions_from_new_generation_rcd_links() {
        let order = HashMap::from([
            parse_rcd_link_name("S95beta").expect("BUG: should parse"),
            parse_rcd_link_name("S10alpha").expect("BUG: should parse"),
        ]);

        assert_eq!(order.get("alpha"), Some(&10));
        assert_eq!(order.get("beta"), Some(&95));
    }

    #[test]
    fn sorts_stop_actions_from_old_generation_k_links() {
        let order = HashMap::from([
            parse_rcd_link_name("K30gamma").expect("BUG: should parse"),
            parse_rcd_link_name("K05alpha").expect("BUG: should parse"),
        ]);

        assert_eq!(order.get("alpha"), Some(&5));
        assert_eq!(order.get("gamma"), Some(&30));
    }

    #[test]
    fn loads_default_config_when_missing() {
        let dir = tempfile::tempdir().expect("BUG: should create temp dir");
        let config = load_service_config(dir.path(), "missing").expect("BUG: should load config");

        assert_eq!(config, ServiceConfig::default());
    }

    #[test]
    fn discovers_and_compares_generation_services() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/removed"), "old-removed");
        write_file(&old_root.join("etc/init.d/shared"), "old-shared");
        write_file(&old_root.join("etc/rc.d/K20removed"), "");
        write_file(&old_root.join("etc/rc.d/S15shared"), "");
        write_file(&old_root.join("etc/rc.d/S99unchanged"), "");
        write_file(&old_root.join("etc/init.d/unchanged"), "same");

        write_file(&new_root.join("etc/init.d/shared"), "new-shared");
        write_file(&new_root.join("etc/init.d/new"), "new-service");
        write_file(&new_root.join("etc/init.d/unchanged"), "same");
        write_file(&new_root.join("etc/rc.d/S05new"), "");
        write_file(&new_root.join("etc/rc.d/S15shared"), "");
        write_file(&new_root.join("etc/rc.d/S99unchanged"), "");

        write_file(
            &new_root.join("etc/init.d.conf/shared.json"),
            r#"{"upgrade_if_status":"always"}"#,
        );

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let removed: Vec<&str> = changes
            .removed
            .iter()
            .map(|change| change.name.as_str())
            .collect();
        let upgraded: Vec<&str> = changes
            .upgraded
            .iter()
            .map(|change| change.name.as_str())
            .collect();
        let new_services: Vec<&str> = changes
            .new
            .iter()
            .map(|change| change.name.as_str())
            .collect();
        let unchanged: Vec<&str> = changes
            .unchanged
            .iter()
            .map(|change| change.name.as_str())
            .collect();

        assert_eq!(removed, vec!["removed"]);
        assert_eq!(upgraded, vec!["shared"]);
        assert_eq!(new_services, vec!["new"]);
        assert_eq!(unchanged, vec!["unchanged"]);
        assert_eq!(
            new.init_services
                .get("shared")
                .expect("BUG: shared should exist")
                .config,
            ServiceConfig {
                upgrade_if_status: crate::service_orchestrator::UpgradeIfStatus::Always,
                ..ServiceConfig::default()
            }
        );
    }

    #[test]
    fn dependency_line_change_classifies_service_as_upgraded() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(
            &old_root.join("etc/init.d/compositor"),
            "START=\"95\"\nDEPENDS_ON=\"/nix/store/old-launcher\"\n",
        );
        write_file(
            &new_root.join("etc/init.d/compositor"),
            "START=\"95\"\nDEPENDS_ON=\"/nix/store/new-launcher\"\n",
        );

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        assert!(changes.new.is_empty());
        assert!(changes.removed.is_empty());
        assert!(changes.unchanged.is_empty());
        assert_eq!(
            changes
                .upgraded
                .iter()
                .map(|change| change.name.as_str())
                .collect::<Vec<_>>(),
            ["compositor"]
        );
    }

    #[test]
    fn empty_old_generation_classifies_all_services_as_new() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let new_root = tempdir.path().join("new");

        write_file(&new_root.join("etc/init.d/alpha"), "alpha-script");
        write_file(&new_root.join("etc/init.d/beta"), "beta-script");
        write_file(&new_root.join("etc/rc.d/S10alpha"), "");
        write_file(&new_root.join("etc/rc.d/S20beta"), "");

        let old =
            discover_generation(Path::new("")).expect("BUG: empty path should return empty gen");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        assert!(old.init_services.is_empty());
        assert!(changes.removed.is_empty());
        assert!(changes.upgraded.is_empty());
        assert!(changes.unchanged.is_empty());

        let new_services: Vec<&str> = changes
            .new
            .iter()
            .map(|change| change.name.as_str())
            .collect();
        assert_eq!(new_services, vec!["alpha", "beta"]);
    }
}

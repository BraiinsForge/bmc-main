// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::BTreeMap;
use std::path::Path;

use crate::types::{Manifest, ManifestPackage, ResolvedPackage};

/// Error type for manifest write operations.
#[derive(Debug, thiserror::Error)]
pub enum WriteManifestError {
    #[error("failed to serialize manifest: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write manifest to {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Build a [`Manifest`] from a slice of resolved packages.
///
/// Each [`ResolvedPackage`] is converted into a [`ManifestPackage`] and keyed
/// by its name. In Stage 1 the cache field is always set to `"local"`.
///
/// TODO(stage-3): carry the actual cache name from `ResolvedPackage` instead
/// of hardcoding `"local"`. The concept doc's manifest uses the cache name
/// from the index.
#[must_use]
pub fn build_manifest(packages: &[ResolvedPackage]) -> Manifest {
    let packages = packages
        .iter()
        .map(|pkg| {
            let entry = ManifestPackage {
                version: pkg.version.clone(),
                cache: "local".into(),
                store_path: pkg.store_path.clone(),
                category: pkg.category.clone(),
                description: pkg.description.clone(),
                upgrade_strategy: pkg.upgrade_strategy.clone(),
                install_strategy: pkg.install_strategy.clone(),
                installed_by: pkg.installed_by.clone(),
                installed_from: pkg.installed_from.clone(),
                pinned: pkg.pinned.clone(),
            };
            (pkg.name.clone(), entry)
        })
        .collect::<BTreeMap<_, _>>();

    Manifest { packages }
}

/// Write a manifest as pretty-printed JSON to `<profile_path>/manifest`.
///
/// Note: `fs::write` itself is not atomic (it truncates and writes in place).
/// Atomicity is provided by the caller (`build_profile`) which writes into a
/// temporary directory and renames it to the final generation path.
pub fn write_manifest(profile_path: &Path, manifest: &Manifest) -> Result<(), WriteManifestError> {
    let json = serde_json::to_string_pretty(manifest).map_err(WriteManifestError::Serialize)?;
    let manifest_path = profile_path.join("manifest");
    std::fs::write(&manifest_path, json).map_err(|source| WriteManifestError::Write {
        path: manifest_path.display().to_string(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InstalledBy, PinStrategy};

    fn test_package(name: &str, store_path: &str) -> ResolvedPackage {
        ResolvedPackage {
            name: name.into(),
            version: "1.0.0".into(),
            store_path: store_path.into(),
            cache_url: "https://cache.example.com".into(),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        }
    }

    #[test]
    fn build_manifest_round_trips() {
        let packages = vec![
            test_package("pkg-a", "/nix/store/aaa-pkg-a-1.0.0"),
            test_package("pkg-b", "/nix/store/bbb-pkg-b-1.0.0"),
        ];

        let manifest = build_manifest(&packages);
        let json =
            serde_json::to_string_pretty(&manifest).expect("BUG: serialization should succeed");
        let parsed: Manifest =
            serde_json::from_str(&json).expect("BUG: deserialization should succeed");

        assert_eq!(parsed.packages.len(), 2);
        assert!(parsed.packages.contains_key("pkg-a"));
        assert!(parsed.packages.contains_key("pkg-b"));
        assert_eq!(
            parsed
                .packages
                .get("pkg-a")
                .expect("BUG: pkg-a should exist")
                .store_path,
            "/nix/store/aaa-pkg-a-1.0.0"
        );
        assert_eq!(
            parsed
                .packages
                .get("pkg-b")
                .expect("BUG: pkg-b should exist")
                .version,
            "1.0.0"
        );
    }

    #[test]
    fn build_manifest_uses_local_cache() {
        let packages = vec![test_package("some-pkg", "/nix/store/xyz-some-pkg-1.0.0")];

        let manifest = build_manifest(&packages);
        let pkg = manifest
            .packages
            .get("some-pkg")
            .expect("BUG: some-pkg should be present in manifest");

        assert_eq!(pkg.cache, "local");
    }

    #[test]
    fn write_manifest_creates_file() {
        let dir = tempfile::tempdir().expect("BUG: should create temp dir");
        let packages = vec![test_package("test-pkg", "/nix/store/abc-test-pkg-1.0.0")];
        let manifest = build_manifest(&packages);

        write_manifest(dir.path(), &manifest).expect("BUG: write_manifest should succeed");

        let manifest_path = dir.path().join("manifest");
        let contents =
            std::fs::read_to_string(&manifest_path).expect("BUG: should read manifest file");
        let parsed: Manifest =
            serde_json::from_str(&contents).expect("BUG: manifest file should contain valid JSON");

        assert_eq!(parsed.packages.len(), 1);
        assert!(parsed.packages.contains_key("test-pkg"));
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::types::{InstalledBy, PackageIndex, PinStrategy, ResolvedPackage};

pub const PACKAGE_INDEX_VERSION: u32 = 1;
pub const FACTORY_INDEX_VERSION: u32 = 1;

#[must_use]
pub fn make_index_url(base_url: &str) -> String {
    format!(
        "{}/nix-package-index.v{}.json",
        base_url.trim_end_matches('/'),
        PACKAGE_INDEX_VERSION,
    )
}

#[must_use]
pub fn make_factory_url(base_url: &str) -> String {
    format!(
        "{}/nix-factory.v{}.json",
        base_url.trim_end_matches('/'),
        FACTORY_INDEX_VERSION,
    )
}

/// Resolve all packages in an index to [`ResolvedPackage`] values.
///
/// Each package entry is given Stage 1 defaults: `installed_by = System`,
/// `installed_from = "local"`, `pinned = None`. Cache metadata from
/// `PackageEntry.cache` is intentionally ignored — store paths are
/// realised through configured Nix substituters.
///
/// This is used by `bmc-nix-cli build-profile` when packages are
/// already present in the local Nix store.
#[must_use]
pub fn resolve_all_from_index(index: &PackageIndex) -> Vec<ResolvedPackage> {
    index
        .packages
        .iter()
        .map(|entry| ResolvedPackage {
            name: entry.name.clone(),
            version: entry.version.clone(),
            store_path: entry.store_path.clone(),
            category: entry.category.clone(),
            description: entry.description.clone(),
            upgrade_strategy: entry.upgrade_strategy.clone(),
            install_strategy: entry.install_strategy.clone(),
            installed_by: InstalledBy::System,
            installed_from: "local".into(),
            pinned: PinStrategy::None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CacheEntry, PackageEntry, PackageIndex, Provenance};

    #[test]
    fn make_index_url_normalizes_configured_base_url() {
        assert_eq!(
            make_index_url("https://cache.braiins.com/v1"),
            "https://cache.braiins.com/v1/nix-package-index.v1.json"
        );
    }

    #[test]
    fn make_factory_url_normalizes_configured_base_url() {
        assert_eq!(
            make_factory_url("https://cache.braiins.com/v1"),
            "https://cache.braiins.com/v1/nix-factory.v1.json"
        );
    }

    #[test]
    fn make_index_url_trims_trailing_slashes() {
        assert_eq!(
            make_index_url("https://cache.braiins.com/v1///"),
            "https://cache.braiins.com/v1/nix-package-index.v1.json"
        );
    }

    #[test]
    fn make_factory_url_trims_trailing_slashes() {
        assert_eq!(
            make_factory_url("https://cache.braiins.com/v1/"),
            "https://cache.braiins.com/v1/nix-factory.v1.json"
        );
    }

    /// Helper: build a minimal `PackageIndex` from given caches and packages.
    fn make_index(caches: Vec<CacheEntry>, packages: Vec<PackageEntry>) -> PackageIndex {
        PackageIndex {
            version: 1,
            provenance: Some(Provenance {
                commit: "test".into(),
            }),
            indexes: vec![],
            caches,
            packages,
        }
    }

    fn default_cache() -> CacheEntry {
        CacheEntry {
            name: "default".into(),
            cache_url: "https://cache.example.com".into(),
            cache_key: "cache.example.com:AAAA".into(),
        }
    }

    fn make_package(name: &str, cache: Option<&str>) -> PackageEntry {
        PackageEntry {
            name: name.into(),
            version: "1.0.0".into(),
            cache: cache.map(Into::into),
            store_path: format!("/nix/store/abc-{name}-1.0.0"),
            category: None,
            description: None,
            upgrade_strategy: None,
            install_strategy: None,
            server_id: String::new(),
        }
    }

    #[test]
    fn resolve_all_from_index_basic() {
        let index = make_index(vec![default_cache()], vec![make_package("hello", None)]);

        let resolved = resolve_all_from_index(&index);

        assert_eq!(resolved.len(), 1);
        let pkg = &resolved[0];
        assert_eq!(pkg.name, "hello");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.store_path, "/nix/store/abc-hello-1.0.0");
        assert_eq!(pkg.installed_from, "local");
        assert_eq!(pkg.pinned, PinStrategy::None);
        assert!(matches!(pkg.installed_by, InstalledBy::System));
    }

    #[test]
    fn resolve_all_ignores_missing_named_cache() {
        let index = make_index(
            vec![default_cache()],
            vec![make_package("broken", Some("nonexistent"))],
        );

        let resolved = resolve_all_from_index(&index);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "broken");
    }

    #[test]
    fn resolve_all_accepts_empty_cache_list() {
        let index = make_index(vec![], vec![make_package("orphan", None)]);

        let resolved = resolve_all_from_index(&index);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "orphan");
    }

    #[test]
    fn resolve_all_multiple_packages() {
        let index = make_index(
            vec![default_cache()],
            vec![
                make_package("alpha", None),
                make_package("beta", None),
                make_package("gamma", None),
            ],
        );

        let resolved = resolve_all_from_index(&index);

        assert_eq!(resolved.len(), 3);
        let names: Vec<&str> = resolved.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);

        for pkg in &resolved {
            assert!(matches!(pkg.installed_by, InstalledBy::System));
            assert_eq!(pkg.installed_from, "local");
        }
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::types::{
    CacheEntry, InstalledBy, PackageEntry, PackageIndex, PinStrategy, ResolvedPackage,
};

/// Errors that can occur when resolving packages from an index.
#[derive(Debug, thiserror::Error)]
pub enum ResolveIndexError {
    #[error("cache '{cache}' not found for package '{package}'")]
    CacheNotFound { package: String, cache: String },
    #[error("no caches defined for package '{package}'")]
    NoCaches { package: String },
}

/// Resolve the cache URL for a package entry.
///
/// If the entry specifies a named cache, look it up in the caches list.
/// If no cache is specified, use the first cache entry as the default.
/// Returns an error if the referenced cache is not found or no caches
/// are defined.
/// Resolved cache information: the URL and the cache name.
struct ResolvedCache {
    url: String,
    name: String,
}

fn resolve_cache(
    entry: &PackageEntry,
    caches: &[CacheEntry],
) -> Result<ResolvedCache, ResolveIndexError> {
    match &entry.cache {
        Some(cache_name) => caches
            .iter()
            .find(|c| c.name == *cache_name)
            .map(|c| ResolvedCache {
                url: c.cache_url.clone(),
                name: c.name.clone(),
            })
            .ok_or_else(|| ResolveIndexError::CacheNotFound {
                package: entry.name.clone(),
                cache: cache_name.clone(),
            }),
        None => caches
            .first()
            .map(|c| ResolvedCache {
                url: c.cache_url.clone(),
                name: c.name.clone(),
            })
            .ok_or_else(|| ResolveIndexError::NoCaches {
                package: entry.name.clone(),
            }),
    }
}

/// Resolve all packages in an index to [`ResolvedPackage`] values.
///
/// Each package entry is paired with its cache URL and given Stage 1
/// defaults: `installed_by = System`, `installed_from = "local"`,
/// `pinned = false`.
///
/// This is used by `bmc-nix-cli build-profile` when packages are
/// already present in the local Nix store.
pub fn resolve_all_from_index(
    index: &PackageIndex,
) -> Result<Vec<ResolvedPackage>, ResolveIndexError> {
    index
        .packages
        .iter()
        .map(|entry| {
            let cache = resolve_cache(entry, &index.caches)?;
            Ok(ResolvedPackage {
                name: entry.name.clone(),
                version: entry.version.clone(),
                store_path: entry.store_path.clone(),
                cache_url: cache.url,
                cache_name: cache.name,
                category: entry.category.clone(),
                description: entry.description.clone(),
                upgrade_strategy: entry.upgrade_strategy.clone(),
                install_strategy: entry.install_strategy.clone(),
                installed_by: InstalledBy::System,
                installed_from: "local".into(),
                pinned: PinStrategy::None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CacheEntry, PackageEntry, PackageIndex, Provenance};

    /// Helper: build a minimal `PackageIndex` from given caches and
    /// packages.
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

        let resolved = resolve_all_from_index(&index)
            .expect("BUG: resolution should succeed with a default cache");

        assert_eq!(resolved.len(), 1);
        let pkg = &resolved[0];
        assert_eq!(pkg.name, "hello");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.store_path, "/nix/store/abc-hello-1.0.0");
        assert_eq!(pkg.cache_url, "https://cache.example.com");
        assert_eq!(pkg.installed_from, "local");
        assert_eq!(pkg.pinned, PinStrategy::None);
        assert!(matches!(pkg.installed_by, InstalledBy::System));
    }

    #[test]
    fn resolve_all_with_named_cache() {
        let extra_cache = CacheEntry {
            name: "extra".into(),
            cache_url: "https://extra-cache.example.com".into(),
            cache_key: "extra.example.com:BBBB".into(),
        };
        let index = make_index(
            vec![default_cache(), extra_cache],
            vec![make_package("world", Some("extra"))],
        );

        let resolved = resolve_all_from_index(&index)
            .expect("BUG: resolution should succeed with named cache");

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].cache_url, "https://extra-cache.example.com");
    }

    #[test]
    fn resolve_all_missing_cache_returns_error() {
        let index = make_index(
            vec![default_cache()],
            vec![make_package("broken", Some("nonexistent"))],
        );

        let err = resolve_all_from_index(&index)
            .expect_err("should fail when named cache does not exist");

        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent"),
            "error should mention the missing cache name: {msg}"
        );
        assert!(
            msg.contains("broken"),
            "error should mention the package name: {msg}"
        );
    }

    #[test]
    fn resolve_all_empty_caches_returns_error() {
        let index = make_index(vec![], vec![make_package("orphan", None)]);

        let err =
            resolve_all_from_index(&index).expect_err("should fail when no caches are defined");

        let msg = err.to_string();
        assert!(
            msg.contains("no caches defined"),
            "error should describe missing caches: {msg}"
        );
        assert!(
            msg.contains("orphan"),
            "error should mention the package name: {msg}"
        );
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

        let resolved = resolve_all_from_index(&index)
            .expect("BUG: resolution should succeed for multiple packages");

        assert_eq!(resolved.len(), 3);
        let names: Vec<&str> = resolved.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);

        // All should share the same cache URL
        for pkg in &resolved {
            assert_eq!(pkg.cache_url, "https://cache.example.com");
            assert!(matches!(pkg.installed_by, InstalledBy::System));
            assert_eq!(pkg.installed_from, "local");
        }
    }
}

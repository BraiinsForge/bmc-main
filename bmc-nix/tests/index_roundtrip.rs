// Copyright (C) 2025  Braiins Systems s.r.o.

//! Round-trip test verifying that JSON produced by `mkIndex.nix`
//! can be parsed by `bmc_nix` types.

/// Round-trip test: read an index JSON fixture (mirrors mkIndex output),
/// parse it with `PackageIndex`, resolve all packages, and verify
/// the results are consistent.
#[test]
fn index_json_round_trip() {
    let json = include_str!("fixtures/test-index.json");
    let index: bmc_nix::types::PackageIndex =
        serde_json::from_str(json).expect("BUG: index JSON should parse");

    assert_eq!(index.version, 1);
    assert!(!index.packages.is_empty());

    // Resolve all packages
    let resolved =
        bmc_nix::index::resolve_all_from_index(&index).expect("BUG: all packages should resolve");

    assert_eq!(resolved.len(), index.packages.len());
    for pkg in &resolved {
        assert!(
            pkg.store_path.starts_with("/nix/store/"),
            "store_path must be a Nix store path: {}",
            pkg.store_path
        );
        assert!(!pkg.name.is_empty());
        assert!(!pkg.version.is_empty());
    }
}

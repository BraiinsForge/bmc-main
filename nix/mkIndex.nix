# mkIndex: Generate nix-package-index.v1.json from a package list.
#
# Takes a list of package entries (each with a `pkg` derivation and metadata)
# and produces a JSON index file that bmc-nix-cli can consume.
{ pkgs, lib }:
{ packages # [ { pkg; name; version; category; description;
  #     upgrade_strategy; install_strategy; cache ? null;
  #     metadata ? { } } ]
  #   metadata is a free-form JSON map; the core entry carries
  #   bmc_version and optionally changelog, and widget entries carry
  #   nested `widget` picker fields and an `assets` map.
, caches ? [ ] # [ { name; cache_url; cache_key; } ]
, indexes ? [ ] # [ "https://..." ] — federated index URLs
, commit ? "" # git commit hash for provenance field
}:
let
  mkPackageEntry = p: {
    inherit (p) name version;
    store_path = "${p.pkg}";
    category = p.category or null;
    description = p.description or null;
    upgrade_strategy = p.upgrade_strategy or null;
    install_strategy = p.install_strategy or null;
  } // lib.optionalAttrs (p ? metadata && p.metadata != null) {
    inherit (p) metadata;
  } // lib.optionalAttrs (p ? cache && p.cache != null) {
    inherit (p) cache;
  };

  indexData = {
    version = 1;
    provenance = if commit != "" then { inherit commit; } else null;
    inherit indexes caches;
    packages = map mkPackageEntry packages;
  };
in
pkgs.writeTextDir "nix-package-index.v1.json" (builtins.toJSON indexData)

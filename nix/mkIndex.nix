# mkIndex: Generate index.json from a package list.
#
# Takes a list of package entries (each with a `pkg` derivation and metadata)
# and produces a JSON index file that bmc-nix-cli can consume.
{ pkgs, lib }:
{ packages # [ { pkg; name; version; category; description;
  #     upgrade_strategy; install_strategy; cache ? null; } ]
, caches ? [ ] # [ { name; cache_url; cache_key; } ]
, indexes ? [ ] # [ "https://..." ] — federated index URLs
, commit ? "" # git commit hash for provenance field
}:
let
  defaultCache =
    if caches != [ ] then (builtins.head caches).name else null;

  mkPackageEntry = p: {
    inherit (p) name version;
    store_path = "${p.pkg}";
    category = p.category or null;
    description = p.description or null;
    upgrade_strategy = p.upgrade_strategy or null;
    install_strategy = p.install_strategy or null;
  } // lib.optionalAttrs (defaultCache != null) {
    cache = p.cache or defaultCache;
  };

  indexData = {
    version = 1;
    provenance = if commit != "" then { inherit commit; } else null;
    inherit indexes caches;
    packages = map mkPackageEntry packages;
  };
in
pkgs.writeTextDir "index.json" (builtins.toJSON indexData)

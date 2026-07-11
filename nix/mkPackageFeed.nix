# mkPackageFeed: Generate nix-package-feed.v1.json from feed entries.
#
# Each entry provides bos_version, download_url, and profile_path.
# This is used for:
# - Local testing (with placeholder URLs)
# - CI integration (with real HTTPS URLs)
{ pkgs }:
{ entries # [ { bos_version; download_url; profile_path; } ]
}:
pkgs.writeTextDir "nix-package-feed.v1.json" (builtins.toJSON {
  version = 1;
  inherit entries;
})

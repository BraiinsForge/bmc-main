# mkFactoryIndex: Generate nix-factory.v1.json from tarball entries.
#
# Each entry provides bos_version, download_url, and profile_path.
# This is used for:
# - Local testing (with placeholder URLs)
# - CI integration (with real HTTPS URLs)
{ pkgs }:
{ tarballs # [ { bos_version; download_url; profile_path; } ]
}:
pkgs.writeTextDir "nix-factory.v1.json" (builtins.toJSON {
  version = 1;
  inherit tarballs;
})

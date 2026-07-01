# Bundle shipped inside the firmware sysupgrade tarball: the on-tarball
# CLI plus the pinned package index the firmware was built against.
{ pkgs }:
{ init-tarball, bmc-nix-cli }:
let
  index = init-tarball.index;
in
pkgs.runCommand "firmware-nix-payload" { } ''
  mkdir -p $out
  cp ${bmc-nix-cli}/bin/bmc-nix-cli $out/
  cp ${index}/nix-package-index.v1.json $out/
''

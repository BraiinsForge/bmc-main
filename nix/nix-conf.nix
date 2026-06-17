# Default /etc/nix/nix.conf contents.
#
# Single source of truth shared by the init tarball (init-artifacts.nix) and
# the core package's recovery activation entry (pkgs/core/nix-conf-activation.nix).
{ pkgs }:
pkgs.writeText "nix.conf" ''
  substituters = https://cache.braiins.com
  # trusted-public-keys = cache.braiins.com:placeholder
  extra-experimental-features = nix-command flakes
''

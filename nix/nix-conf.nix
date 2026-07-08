# Default /etc/nix/nix.conf contents.
#
# Single source of truth, written by the core package's recovery
# activation entry (pkgs/core/nix-conf-activation.nix).
{ pkgs }:
pkgs.writeText "nix.conf" ''
  substituters = https://cache.braiins.com
  # trusted-public-keys = cache.braiins.com:placeholder
  extra-experimental-features = nix-command flakes
  # nix-store --realise must fsync realized store path contents (the
  # SQLite DB is fsynced by default, contents are not).
  fsync-store-paths = true
''

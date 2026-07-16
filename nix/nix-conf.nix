# Default /etc/nix/nix.conf contents.
#
# Single source of truth, written by the core package's recovery
# activation entry (pkgs/core/nix-conf-activation.nix).
{ pkgs }:
pkgs.writeText "nix.conf" ''
  substituters = https://downloads.braiinsforge.com
  trusted-public-keys = downloads.braiinsforge.com-1:4XDOIc61MHtHeIOVrgNOfgzZHt4RCPfqWGDt5PwsLeU=
  extra-experimental-features = nix-command flakes
  # nix-store --realise must fsync realized store path contents (the
  # SQLite DB is fsynced by default, contents are not).
  fsync-store-paths = true
''

# Recovery activation entry: recreate /etc/nix/nix.conf with default contents
# when it is missing, so `nix` keeps working if the file ever disappears.
#
# Create-only: an existing (possibly user-modified) nix.conf is left untouched.
# NIX_CONF_ACTIVATION_ROOT relocates the target for tests.
{ pkgs, nixConf }:
pkgs.writeTextFile {
  name = "nix-conf-activation";
  executable = true;
  destination = "/bin/nix-conf-activation";
  text = ''
    #!/bin/sh
    set -e

    root="''${NIX_CONF_ACTIVATION_ROOT:-/}"
    case "$root" in
      /) target=/etc/nix/nix.conf ;;
      *) target="$root/etc/nix/nix.conf" ;;
    esac

    if [ ! -e "$target" ]; then
        mkdir -p "$(dirname "$target")"
        cp "${nixConf}" "$target.tmp"
        chmod 644 "$target.tmp"
        mv -Tf "$target.tmp" "$target"
    fi
  '';
}

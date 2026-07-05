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
        # The temp file's data must be durable before the publishing
        # rename: a crash can then only yield a missing nix.conf
        # (recreated on the next boot's activation) or a complete one —
        # never an existing-but-corrupt file that this create-only
        # entry would refuse to repair. Prefer dd conv=fsync (reports
        # writeback errors); conv= is an optional BusyBox feature, so
        # probe for it first. dd parses operands before opening of=,
        # so a failed probe that left no probe file means the conv=
        # operand was rejected -> fall back to cp + sync. A failed
        # probe that DID create the probe file is a real write/fsync
        # error -> abort; and a failure of the real dd write below
        # aborts too (set -e), never retrying through the degraded
        # path. The rename itself is deliberately not fsynced: losing
        # it degrades to "file absent", which self-heals.
        probe="$target.probe"
        rm -f "$probe"
        if dd if=/dev/null of="$probe" conv=fsync 2>/dev/null; then
            rm -f "$probe"
            dd if="${nixConf}" of="$target.tmp" conv=fsync 2>/dev/null
        elif [ ! -e "$probe" ]; then
            cp "${nixConf}" "$target.tmp"
            sync
        else
            rm -f "$probe"
            echo "nix-conf-activation: dd conv=fsync probe failed" >&2
            exit 1
        fi
        chmod 644 "$target.tmp"
        mv -Tf "$target.tmp" "$target"
    fi
  '';
}

# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

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
        # entry would refuse to repair. `sync` flushes the temp file's
        # data before the rename. The rename itself is deliberately not
        # fsynced: losing it degrades to "file absent", which self-heals
        # on the next activation.
        cp "${nixConf}" "$target.tmp"
        sync
        chmod 644 "$target.tmp"
        mv -Tf "$target.tmp" "$target"
    fi
  '';
}

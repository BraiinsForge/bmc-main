#!/bin/sh
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

# Exercises the nix-conf recovery activation script against fake roots.
# The real /etc is never touched. Argument $1 is the script to run.
# When NIX_CONF_ACTIVATION_SHELL is set, the script is run through that shell.
#
# Manual use from the repository root:
#
#   system=$(nix eval --impure --raw --expr builtins.currentSystem)
#   nix build -L ".#legacyPackages.${system}.deck-packages.core.pkg.tests.activation"
set -eu

script="$1"
tmp="${TMPDIR:-/tmp}/nix-conf-activation-test-$$"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT

run_activation() {
    root="$1"
    if [ -n "${NIX_CONF_ACTIVATION_SHELL:-}" ]; then
        NIX_CONF_ACTIVATION_ROOT="$root" "$NIX_CONF_ACTIVATION_SHELL" "$script"
    else
        NIX_CONF_ACTIVATION_ROOT="$root" "$script"
    fi
}

# A missing nix.conf is restored with the default contents.
root="$tmp/restore"
mkdir -p "$root"
run_activation "$root"
conf="$root/etc/nix/nix.conf"
test -f "$conf" || fail "nix.conf was not created"
grep -F 'extra-experimental-features = nix-command flakes' "$conf" >/dev/null 2>&1 \
    || fail "restored nix.conf missing the default experimental-features line"
grep -F 'fsync-store-paths = true' "$conf" >/dev/null 2>&1 \
    || fail "restored nix.conf missing fsync-store-paths"
grep -F 'narinfo-cache-negative-ttl = 0' "$conf" >/dev/null 2>&1 \
    || fail "restored nix.conf missing narinfo-cache-negative-ttl"

# An existing (modified) nix.conf is left byte-for-byte untouched.
root="$tmp/keep"
mkdir -p "$root/etc/nix"
printf 'custom = setting\n' >"$root/etc/nix/nix.conf"
run_activation "$root"
test "$(cat "$root/etc/nix/nix.conf")" = "custom = setting" \
    || fail "existing nix.conf was overwritten"

echo "OK"

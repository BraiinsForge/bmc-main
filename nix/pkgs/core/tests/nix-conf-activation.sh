#!/bin/sh
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

# An existing (modified) nix.conf is left byte-for-byte untouched.
root="$tmp/keep"
mkdir -p "$root/etc/nix"
printf 'custom = setting\n' >"$root/etc/nix/nix.conf"
run_activation "$root"
test "$(cat "$root/etc/nix/nix.conf")" = "custom = setting" \
    || fail "existing nix.conf was overwritten"

echo "OK"

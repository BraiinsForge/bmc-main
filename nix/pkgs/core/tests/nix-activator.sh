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

# Exercises activate_profile of the nix-activator against fake profile
# dirs. The real system is never touched. Argument $1 is the activator
# script, which is sourced (it only defines functions and START).
# Fake generation entrypoints run through NIX_ACTIVATOR_TEST_SHELL
# (default /bin/sh).
#
# Manual use from the repository root:
#
#   system=$(nix eval --impure --raw --expr builtins.currentSystem)
#   nix build -L ".#legacyPackages.${system}.deck-packages.core.pkg.tests.activator"
set -eu

activator="$1"
shell="${NIX_ACTIVATOR_TEST_SHELL:-/bin/sh}"
tmp="${TMPDIR:-/tmp}/nix-activator-test-$$"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT

. "$activator"

LOG() {
    echo "nix-activator: $*" >&2
}

mkdir -p "$tmp"
printf 'test-1\n' >"$tmp/bos_version"

NIX_ACTIVATOR_BOS_VERSION_FILE="$tmp/bos_version"

write_entrypoint() {
    mkdir -p "$1/core/activation"
    printf '#!%s\n%s\n' "$shell" "$2" >"$1/core/activation/entrypoint"
    chmod 755 "$1/core/activation/entrypoint"
}

# A staged entrypoint that flips 'current' and then fails: 'current' is
# restored to the previous generation, the fallback re-runs it, and the
# marker is kept for a retry.
profile="$tmp/restore"
mkdir -p "$profile"
write_entrypoint "$profile/1-link" "touch \"$profile/old-ran\""
write_entrypoint "$profile/2-link" "ln -sfn 2-link \"$profile/current\"; exit 1"
ln -s 1-link "$profile/current"
ln -s 2-link "$profile/next.test-1"

NIX_ACTIVATOR_PROFILE_DIR="$profile"
activate_profile || fail "activation with a working fallback should succeed"

test "$(readlink "$profile/current")" = "1-link" \
    || fail "current was not restored to the previous generation"
test -f "$profile/old-ran" \
    || fail "fallback did not re-run the previous generation"
test -L "$profile/next.test-1" \
    || fail "retry marker was removed on failure"

# A staged entrypoint that fails without touching 'current': no restore
# is needed, the fallback re-runs the previous generation, and the
# marker is kept.
profile="$tmp/untouched"
mkdir -p "$profile"
write_entrypoint "$profile/1-link" "touch \"$profile/old-ran\""
write_entrypoint "$profile/2-link" "exit 1"
ln -s 1-link "$profile/current"
ln -s 2-link "$profile/next.test-1"

NIX_ACTIVATOR_PROFILE_DIR="$profile"
activate_profile || fail "activation with a working fallback should succeed"

test "$(readlink "$profile/current")" = "1-link" \
    || fail "current changed although the entrypoint never touched it"
test -f "$profile/old-ran" \
    || fail "fallback did not re-run the previous generation"
test -L "$profile/next.test-1" \
    || fail "retry marker was removed on failure"

# No prior 'current': a staged entrypoint that creates it and then
# fails leaves no 'current' behind, so the initial-initialization
# fallback (find_latest_link) applies. The fake entrypoint flips only
# on its first run, mirroring a real activation that reached the final
# flip once and fails earlier on the retry.
profile="$tmp/no-current"
mkdir -p "$profile"
write_entrypoint "$profile/2-link" "
if [ -e \"$profile/staged-ran\" ]; then exit 1; fi
touch \"$profile/staged-ran\"
ln -sfn 2-link \"$profile/current\"
exit 1"
ln -s 2-link "$profile/next.test-1"

NIX_ACTIVATOR_PROFILE_DIR="$profile"
if activate_profile; then
    fail "activation should fail when every entrypoint fails"
fi

test ! -L "$profile/current" \
    || fail "current left behind by the failed activation"
test -f "$profile/staged-ran" \
    || fail "staged entrypoint never ran"
test -L "$profile/next.test-1" \
    || fail "retry marker was removed on failure"

# Plain boot: no staged marker, 'current' exists. The current
# generation's entrypoint runs (without PROFILE_OLD_GENERATION) and
# 'current' stays put.
profile="$tmp/plain"
mkdir -p "$profile"
write_entrypoint "$profile/1-link" "
test -z \"\${PROFILE_OLD_GENERATION:-}\" || exit 1
touch \"$profile/current-ran\""
ln -s 1-link "$profile/current"

NIX_ACTIVATOR_PROFILE_DIR="$profile"
activate_profile || fail "plain boot activation reported failure"

test "$(readlink "$profile/current")" = "1-link" \
    || fail "current changed on a plain boot"
test -f "$profile/current-ran" \
    || fail "current generation entrypoint did not run on a plain boot"

# A staged entrypoint that flips 'current' and succeeds: the marker is
# consumed and no fallback runs.
profile="$tmp/success"
mkdir -p "$profile"
write_entrypoint "$profile/1-link" "touch \"$profile/old-ran\""
write_entrypoint "$profile/2-link" "ln -sfn 2-link \"$profile/current\""
ln -s 1-link "$profile/current"
ln -s 2-link "$profile/next.test-1"

NIX_ACTIVATOR_PROFILE_DIR="$profile"
activate_profile || fail "successful staged activation reported failure"

test "$(readlink "$profile/current")" = "2-link" \
    || fail "current was not advanced to the new generation"
test ! -e "$profile/next.test-1" \
    || fail "marker was not consumed on success"
test ! -e "$profile/old-ran" \
    || fail "fallback ran after a successful staged activation"

echo "OK"

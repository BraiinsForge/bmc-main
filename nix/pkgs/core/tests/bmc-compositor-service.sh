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

set -eu

script="$1"
busybox="$2"
expected_launcher="$3"
expected_host="$4"
expected_system_config="$5"
tmp="${TMPDIR:-/tmp}/bmc-compositor-service-test-$$"
root="$tmp/root"
calls="$tmp/calls"
fail_command=

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

cleanup() {
    "$busybox" rm -rf "$tmp"
}
trap cleanup EXIT

record() {
    printf '%s\n' "$*" >>"$calls"
}

map_path() {
    printf '%s%s' "$root" "$1"
}

mkdir() {
    record "mkdir $*"
    [ "${1:-}" != -p ] || shift
    for path in "$@"; do
        [ "$fail_command" != mkdir ] || [ "$path" != /mnt/data/bmc/cache ] || return 1
        "$busybox" mkdir -p "$(map_path "$path")"
    done
}

rm() {
    record "rm $*"
    [ "$fail_command" != rm ] || return 1
    [ "${1:-}" != -rf ] || shift
    for path in "$@"; do
        "$busybox" rm -rf "$(map_path "$path")"
    done
}

chmod() {
    record "chmod $*"
    [ "$fail_command" != chmod ] || return 1
    mode="$1"
    shift
    for path in "$@"; do
        "$busybox" chmod "$mode" "$(map_path "$path")"
    done
}

logger() {
    record "logger $*"
}

stop() {
    record stop
}

start() {
    record start
}

procd_open_instance() {
    record procd_open_instance
}

procd_set_param() {
    record "procd_set_param $*"
}

procd_close_instance() {
    record procd_close_instance
}

assert_script_contains() {
    needle="$1"
    message="$2"
    grep -F "$needle" "$script" >/dev/null 2>&1 || fail "$message"
}

assert_best_effort_failure() {
    fail_command="$1"
    expected_message="$2"
    : >"$calls"
    start_service
    grep -F "logger -t bmc-compositor $expected_message" "$calls" >/dev/null 2>&1 \
        || fail "$1 failure was not logged"
    grep -Fx procd_open_instance "$calls" >/dev/null 2>&1 \
        || fail "$1 failure prevented procd registration"
    fail_command=
}

env_calls=$(grep -c 'procd_set_param env' "$script" || :)
[ "$env_calls" -eq 1 ] || fail "generated service must emit one procd env call"
assert_script_contains '"BMC_SERVICE_NAME=bmc-compositor"' \
    "generated service does not publish its own name to the daemon"
assert_script_contains '"MESA_SHADER_CACHE_MAX_SIZE=16M"' \
    "generated service is missing MESA_SHADER_CACHE_MAX_SIZE=16M"
assert_script_contains '"XDG_CACHE_HOME=/mnt/data/bmc/cache"' \
    "generated service is missing XDG_CACHE_HOME=/mnt/data/bmc/cache"
assert_script_contains 'mkdir -p /tmp/runtime' \
    "generated service lost XDG runtime-directory setup"
assert_script_contains 'rm -rf /.cache/mesa_shader_cache' \
    "generated service does not remove the legacy Mesa cache"
assert_script_contains 'mkdir -p /mnt/data/bmc/cache' \
    "generated service does not create the persistent cache directory"
assert_script_contains 'chmod 0700 /mnt/data/bmc/cache' \
    "generated service does not secure the persistent cache directory"
assert_script_contains "DEPENDS_ON=\"$expected_launcher $expected_host $expected_system_config\"" \
    "generated service does not depend on the wasm launcher, host, and system config"

# shellcheck source=/dev/null
. "$script"

"$busybox" mkdir -p "$root/.cache/mesa_shader_cache"
"$busybox" touch "$root/.cache/mesa_shader_cache/entry"
"$busybox" touch "$root/.cache/unrelated"

: >"$calls"
reload_service
[ "$(cat "$calls")" = "$(printf 'stop\nstart')" ] \
    || fail "reload_service must stop then start exactly once"

: >"$calls"
start_service

[ ! -e "$root/.cache/mesa_shader_cache" ] \
    || fail "legacy Mesa cache survived preStart"
[ -f "$root/.cache/unrelated" ] \
    || fail "preStart removed unrelated root-cache content"
[ -d "$root/mnt/data/bmc/cache" ] \
    || fail "persistent cache directory was not created"
[ "$("$busybox" stat -c '%a' "$root/mnt/data/bmc/cache")" = 700 ] \
    || fail "persistent cache directory mode is not 0700"
[ -d "$root/tmp/runtime" ] \
    || fail "XDG runtime directory was not created"
grep -Fx procd_open_instance "$calls" >/dev/null 2>&1 \
    || fail "successful setup did not register a procd instance"

: >"$calls"
start_service
grep -Fx procd_open_instance "$calls" >/dev/null 2>&1 \
    || fail "idempotent cleanup did not register a procd instance"

assert_best_effort_failure rm "failed to remove legacy Mesa shader cache"
assert_best_effort_failure mkdir "failed to create persistent cache directory"
assert_best_effort_failure chmod "failed to secure persistent cache directory"

echo "OK"

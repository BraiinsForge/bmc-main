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
tmp="${TMPDIR:-/tmp}/bos-avahi-test-$$"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

assert_file_contains() {
    file="$1"
    needle="$2"
    grep -F "$needle" "$file" >/dev/null 2>&1 || fail "$file missing $needle"
}

make_root() {
    root="$tmp/$1"
    mkdir -p "$root/etc"
    printf 'root:x:0:\n' >"$root/etc/group"
    printf 'root:x:0:0:root:/root:/bin/sh\n' >"$root/etc/passwd"
    echo "$root"
}

run_activation() {
    root="$1"
    BOS_AVAHI_ROOT="$root" "$script"
}

cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT

root="$(make_root bmm1)"
printf 'stm32mp157c-ii2-bmm1\n' >"$root/etc/bos_platform"
run_activation "$root"
assert_file_contains "$root/etc/group" 'avahi:x:100:'
assert_file_contains "$root/etc/passwd" 'avahi:x:100:100:avahi:/var/run/avahi-daemon:/bin/false'
assert_file_contains "$root/etc/avahi/services/bos.service" '<type>_http._tcp</type>'
assert_file_contains "$root/etc/avahi/services/bos.service" '<subtype>_bos._sub._http._tcp</subtype>'
assert_file_contains "$root/etc/avahi/services/bos.service" '<port>80</port>'

root="$(make_root bfm1)"
printf 'stm32mp157c-ii4-bfm1\n' >"$root/etc/bos_platform"
run_activation "$root"
test -f "$root/etc/avahi/services/bos.service" || fail "bfm1 should create bos.service"

root="$(make_root bmc1)"
mkdir -p "$root/etc/avahi/services"
printf 'stale\n' >"$root/etc/avahi/services/bos.service"
printf 'stm32mp157c-ii3-bmc1\n' >"$root/etc/bos_platform"
run_activation "$root"
test ! -e "$root/etc/avahi/services/bos.service" || fail "non-miner should remove bos.service"

root="$(make_root existing)"
printf 'avahi:x:123:\n' >>"$root/etc/group"
printf 'avahi:x:123:123:custom:/tmp:/bin/false\n' >>"$root/etc/passwd"
printf 'stm32mp157c-ii2-bmm1\n' >"$root/etc/bos_platform"
run_activation "$root"
assert_file_contains "$root/etc/group" 'avahi:x:123:'
assert_file_contains "$root/etc/passwd" 'avahi:x:123:123:custom:/tmp:/bin/false'

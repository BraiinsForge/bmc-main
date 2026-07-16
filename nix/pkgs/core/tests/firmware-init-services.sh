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

# Exercises the firmware service bridge against fake roots. The real /etc and
# /rom are never touched. Argument $1 is the activation script to run.
set -eu

script="$1"
shell="${FIRMWARE_INIT_SERVICES_TEST_SHELL:-/bin/sh}"
tmp="${TMPDIR:-/tmp}/firmware-init-services-test-$$"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT

write_legacy_bmc() {
    root="$1"
    mkdir -p "$root/etc/init.d" "$root/etc/rc.d"
    cat >"$root/etc/init.d/bmc" <<'EOF'
#!/bin/sh
printf '%s\n' "$1" >>"$FIRMWARE_INIT_SERVICES_ROOT/bmc-calls"
EOF
    chmod 755 "$root/etc/init.d/bmc"
}

run_activation() {
    root="$1"
    FIRMWARE_INIT_SERVICES_ROOT="$root" "$shell" "$script"
}

# Transitional firmware must stop and disable the legacy compositor before
# installing the profile-managed services.
root="$tmp/transitional"
mkdir -p "$root/rom/etc/init.d" "$root/rom/etc/rc.d"
write_legacy_bmc "$root"
run_activation "$root"
printf 'stop\ndisable\n' >"$root/expected-calls"
cmp "$root/expected-calls" "$root/bmc-calls" \
    || fail "legacy bmc was not stopped and disabled in order"
test -x "$root/etc/init.d/nix-activator" \
    || fail "transition activator was not installed"
test -L "$root/etc/rc.d/S91nix-activator" \
    || fail "transition activator was not enabled"

# Firmware with its own activator is already past the transition and must not
# touch the legacy service path.
root="$tmp/bundled"
mkdir -p "$root/rom/etc/init.d" "$root/rom/etc/rc.d"
touch "$root/rom/etc/init.d/nix-activator"
chmod 755 "$root/rom/etc/init.d/nix-activator"
write_legacy_bmc "$root"
run_activation "$root"
test ! -e "$root/bmc-calls" \
    || fail "legacy bmc was touched on firmware with a bundled activator"

echo "OK"

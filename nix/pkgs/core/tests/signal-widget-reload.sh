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
tmp="${TMPDIR:-/tmp}/signal-widget-reload-test-$$"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin"

cat >"$tmp/bin/ubus" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$UBUS_ARGS"
exit "$UBUS_EXIT"
EOF
chmod +x "$tmp/bin/ubus"

cat >"$tmp/expected" <<'EOF'
call
service
signal
{ "name": "bmc-compositor", "signal": 28 }
EOF

UBUS_ARGS="$tmp/success.args" UBUS_EXIT=0 PATH="$tmp/bin:$PATH" "$script"
cmp "$tmp/expected" "$tmp/success.args"

UBUS_ARGS="$tmp/failure.args" UBUS_EXIT=1 PATH="$tmp/bin:$PATH" "$script"
cmp "$tmp/expected" "$tmp/failure.args"

# A generation without ubus on PATH (exotic rollback) must not fail either.
mkdir -p "$tmp/nobin"
PATH="$tmp/nobin" "$script"

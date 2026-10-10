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

step="$1"
worker="$2"
orchestrator="$3"
tmp="${TMPDIR:-/tmp}/widget-autoinstall-test-$$"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/bin" "$tmp/nobin"

# ubus: logs each call; `service list` reports a running instance for its
# first $UBUS_RUNNING_LISTS calls, then prints $UBUS_LIST and exits
# $UBUS_LIST_EXIT. Every other call exits $UBUS_EXIT.
cat >"$tmp/bin/ubus" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$UBUS_LOG"
if [ "$3" = list ]; then
  if [ "$(grep -c "service list" "$UBUS_LOG")" -le "${UBUS_RUNNING_LISTS:-0}" ]; then
    cat "$UBUS_RUNNING"
    exit 0
  fi
  cat "$UBUS_LIST"
  exit "${UBUS_LIST_EXIT:-0}"
fi
exit "${UBUS_EXIT:-0}"
EOF
cat >"$tmp/bin/logger" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$LOGGER_LOG"
EOF
cat >"$tmp/bin/sleep" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$SLEEP_LOG"
EOF
chmod +x "$tmp/bin/ubus" "$tmp/bin/logger" "$tmp/bin/sleep"

printf '{ "x": { "instances": { "main": { "running": true } } } }\n' >"$tmp/running"
printf '{ }\n' >"$tmp/idle"
cat >"$tmp/list-only" <<'EOF'
call service list {"name":"bmc-widget-autoinstall"}
EOF
cat "$tmp/list-only" - >"$tmp/registered" <<EOF
call service delete {"name":"bmc-widget-autoinstall"}
call service set {
  "name": "bmc-widget-autoinstall",
  "instances": {
    "main": {
      "command": ["$worker", "/p/bmc/7-link"],
      "stdout": true,
      "stderr": true
    }
  }
}
EOF

run() {
    : >"$tmp/ubus.log"
    : >"$tmp/logger.log"
    : >"$tmp/cli.args"
    : >"$tmp/cli.path"
    : >"$tmp/sleep.log"
    UBUS_LOG="$tmp/ubus.log" UBUS_RUNNING="$tmp/running" LOGGER_LOG="$tmp/logger.log" \
        SLEEP_LOG="$tmp/sleep.log" \
        CLI_ARGS="$tmp/cli.args" CLI_PATH="$tmp/cli.path" \
        PATH="$tmp/bin:$PATH" "$@"
}

# Step: a running worker is left alone (re-registering would kill it).
UBUS_LIST="$tmp/running" PROFILE_NEW_GENERATION=/p/bmc/7-link run ash "$step"
cmp "$tmp/list-only" "$tmp/ubus.log"

# Step: an unanswered list may hide a running worker too.
UBUS_LIST="$tmp/idle" UBUS_LIST_EXIT=1 PROFILE_NEW_GENERATION=/p/bmc/7-link run ash "$step"
cmp "$tmp/list-only" "$tmp/ubus.log"
grep -q 'procd unreachable' "$tmp/logger.log"

# Step: replaces the worker, registered with the new generation.
UBUS_LIST="$tmp/idle" PROFILE_NEW_GENERATION=/p/bmc/7-link run ash "$step"
cmp "$tmp/registered" "$tmp/ubus.log"

# Step: a failed registration is logged and never fails the activation.
UBUS_LIST="$tmp/idle" UBUS_EXIT=1 PROFILE_NEW_GENERATION=/p/bmc/7-link run ash "$step"
grep -q 'failed to register' "$tmp/logger.log"

# Step: no ubus (nor anything else) on PATH still exits 0.
PATH="$tmp/nobin" "$(command -v ash)" "$step"

# Worker: waits on the orchestrator core registers, passes the pinned
# triple and the generation, reaches nix-store, logs the status.
UBUS_LIST="$tmp/idle" CLI_EXIT=0 run ash "$worker" /p/bmc/7-link
cat >"$tmp/expected" <<'EOF'
add-packages
--if-missing
--if-current
/p/bmc/7-link
--profile-dir
/p/bmc
--name
widget-a
--version
1.0.0
--store-path
/nix/store/00000000000000000000000000000000-widget-a
EOF
grep -qF "service list {\"name\":\"$orchestrator\"}" "$tmp/ubus.log"
cmp "$tmp/expected" "$tmp/cli.args"
grep -q ':/run/current-profile/bin:' "$tmp/cli.path"
grep -q 'exited with status 0' "$tmp/logger.log"

# Worker: a failing install is logged and still exits 0.
UBUS_LIST="$tmp/idle" CLI_EXIT=2 run ash "$worker" /p/bmc/7-link
grep -q 'exited with status 2' "$tmp/logger.log"

# Worker: polls while the orchestrator runs and installs once it is idle.
UBUS_LIST="$tmp/idle" UBUS_RUNNING_LISTS=3 CLI_EXIT=0 run ash "$worker" /p/bmc/7-link
test "$(grep -c "service list" "$tmp/ubus.log")" = 4
printf '2\n2\n2\n' | cmp - "$tmp/sleep.log"
cmp "$tmp/expected" "$tmp/cli.args"

# Worker: gives up when the orchestrator never finishes, installs nothing.
UBUS_LIST="$tmp/running" CLI_EXIT=0 run ash "$worker" /p/bmc/7-link
test ! -s "$tmp/cli.args"
grep -q 'not seen idle after 600s' "$tmp/logger.log"
test "$(grep -cx 2 "$tmp/sleep.log")" = 300
test "$(grep -c '' "$tmp/sleep.log")" = 300

# Worker: an unanswered list never counts as an idle orchestrator.
UBUS_LIST="$tmp/idle" UBUS_LIST_EXIT=1 CLI_EXIT=0 run ash "$worker" /p/bmc/7-link
test ! -s "$tmp/cli.args"

#!/usr/bin/env bash
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

# LED effects test script — exercises all LedEffect variants, brightness,
# enable/disable, and temp-effect expiry via the LedTestService gRPC endpoint.
#
# Usage:
#   ./led-test-effects.sh DEVICE_IP:PORT PASSWORD
#
# Requires: grpcurl

set -euo pipefail

if [[ $# -lt 2 ]]; then
    echo "Usage: $0 DEVICE_IP:PORT PASSWORD" >&2
    exit 1
fi

ADDR=$1
PASSWORD=$2
DELAY=3

echo "=== Authenticating ==="
COOKIE=$(grpcurl -plaintext -v \
    -d "{\"password\": \"$PASSWORD\"}" \
    "$ADDR" \
    braiins.bmc.web.AuthenticationService/Login 2>&1 \
    | sed -n 's/^set-cookie: \(session_id=[^;]*\).*/\1/p')

if [[ -z $COOKIE ]]; then
    echo "ERROR: authentication failed" >&2
    exit 1
fi
echo "OK"

grpc() {
    local method=$1
    shift
    grpcurl -plaintext \
        -H "cookie: $COOKIE" \
        ${1:+-d "$1"} \
        "$ADDR" \
        "braiins.bmc.web.LedTestService/$method" \
        >/dev/null
}

# Print label, execute grpc command, then pause for visual verification.
run() {
    local label=$1
    shift
    echo "--- $label"
    grpc "$@"
    sleep "$DELAY"
}

# ─── Effect types ─────────────────────────────────────────────────────

echo ""
echo "=== Effect types ==="

run "Solid red (static)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_SOLID","color":{"r":255,"g":0,"b":0}}'

run "Solid green (static)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_SOLID","color":{"r":0,"g":255,"b":0}}'

run "Solid blue (static)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_SOLID","color":{"r":0,"g":0,"b":255}}'

run "Solid white (same as PreviewScene)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_SOLID","color":{"r":255,"g":255,"b":255}}'

run "KnightRider violet, 1s cycle (same as DeviceInitializing)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_KNIGHT_RIDER","color":{"r":107,"g":80,"b":255},"period_ms":1000}'

run "KnightRider orange, 1s cycle (same as DownloadOrUpgradeStarted)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_KNIGHT_RIDER","color":{"r":255,"g":122,"b":13},"period_ms":1000}'

run "Breathe green, 4s cycle (same as PriceUp)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_BREATHE","color":{"r":0,"g":255,"b":0},"period_ms":4000}'

run "Breathe red, 4s cycle (same as PriceDown)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_BREATHE","color":{"r":255,"g":0,"b":0},"period_ms":4000}'

run "Breathe orange, 4s cycle (same as ClockAlarm)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_BREATHE","color":{"r":255,"g":122,"b":13},"period_ms":4000}'

run "Chase blue, 2s cycle" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_CHASE","color":{"r":0,"g":0,"b":255},"period_ms":2000}'

run "Scan cyan, 1.5s cycle" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_SCAN","color":{"r":0,"g":255,"b":255},"period_ms":1500}'

run "Snake magenta, 1s cycle" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_SNAKE","color":{"r":255,"g":0,"b":255},"period_ms":1000}'

run "None (LEDs off)" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_NONE"}'

# ─── Brightness ───────────────────────────────────────────────────────

echo ""
echo "=== Brightness ==="

grpc SetEffect '{"effect":"LED_EFFECT_TYPE_SOLID","color":{"r":255,"g":255,"b":255}}'

run "Full brightness (1.0)" \
    SetBrightness '{"brightness":1.0}'

run "Half brightness (0.5)" \
    SetBrightness '{"brightness":0.5}'

run "Low brightness (0.1)" \
    SetBrightness '{"brightness":0.1}'

run "Zero brightness (0.0)" \
    SetBrightness '{"brightness":0.0}'

run "Restore full brightness" \
    SetBrightness '{"brightness":1.0}'

grpc SetEffect '{"effect":"LED_EFFECT_TYPE_NONE"}'

# ─── Enable / Disable ────────────────────────────────────────────────

echo ""
echo "=== Enable / Disable ==="

echo "--- Start KnightRider violet"
grpc SetEffect '{"effect":"LED_EFFECT_TYPE_KNIGHT_RIDER","color":{"r":107,"g":80,"b":255},"period_ms":1000}'
sleep "$DELAY"

run "Disable — LEDs off" \
    Disable

run "Enable — animation resumes" \
    Enable

run "Clear" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_NONE"}'

# ─── Temp effects ────────────────────────────────────────────────────

echo ""
echo "=== Temp effects ==="

echo "--- Temp solid green 2s on static background (should expire to off)"
grpc SetEffect '{"effect":"LED_EFFECT_TYPE_NONE"}'
grpc SetEffect '{"effect":"LED_EFFECT_TYPE_SOLID","color":{"r":0,"g":255,"b":0},"duration_ms":2000}'
sleep "$DELAY"

echo "--- Temp solid red 2s on animated background (should return to KnightRider)"
grpc SetEffect '{"effect":"LED_EFFECT_TYPE_KNIGHT_RIDER","color":{"r":107,"g":80,"b":255},"period_ms":1000}'
sleep 1
grpc SetEffect '{"effect":"LED_EFFECT_TYPE_SOLID","color":{"r":255,"g":0,"b":0},"duration_ms":2000}'
sleep "$DELAY"

run "Clear" \
    SetEffect '{"effect":"LED_EFFECT_TYPE_NONE"}'

# ─── Done ─────────────────────────────────────────────────────────────

echo ""
echo "=== All tests complete ==="

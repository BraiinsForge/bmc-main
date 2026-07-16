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

[ -t 1 ] || return 0
[ -n "${BMC_VIRT_BANNER_SHOWN:-}" ] && return 0
export BMC_VIRT_BANNER_SHOWN=1

# -- Colors (use printf to produce real escape bytes; ash won't expand \033 in variables) --
ESC=$(printf '\033')
HEAD="${ESC}[33;1m"
DIM="${ESC}[2m"
RESET="${ESC}[0m"

# -- Architecture --
ARCH=$(uname -m)

# -- Acceleration mode --
# QEMU with KVM/HVF exposes "KVM" or "HVF" in cpuinfo/DMI; TCG does not.
if grep -qs 'KVM' /sys/class/dmi/id/sys_vendor 2>/dev/null \
    || grep -qs 'KVM' /proc/cpuinfo 2>/dev/null; then
    ACCEL="KVM"
elif grep -qs 'Apple' /sys/class/dmi/id/sys_vendor 2>/dev/null; then
    ACCEL="HVF"
else
    ACCEL="TCG (slow)"
fi

# -- Kernel --
KVER=$(uname -r)

# -- DRM devices --
DRM=""
for card in /sys/class/drm/card[0-9]; do
    [ -d "$card" ] || continue
    name=$(basename "$card")
    driver=$(basename "$(readlink "$card/device/driver" 2>/dev/null)" 2>/dev/null)
    [ -z "$driver" ] && driver="unknown"
    DRM="${DRM:+$DRM, }${name}(${driver})"
done
[ -z "$DRM" ] && DRM="none"

# -- WiFi --
if [ -d /sys/devices/virtual/mac80211_hwsim/hwsim0 ]; then
    # Check if hostapd is running (AP mode)
    if pgrep -x hostapd >/dev/null 2>&1; then
        SSID=$(uci -q get wireless.default_radio0.ssid 2>/dev/null)
        WIFI="up – AP \"${SSID:-?}\""
    else
        WIFI="up – hostapd not running"
    fi
else
    WIFI="not available"
fi

# -- SPI --
if [ -c /dev/spidev0.0 ]; then
    SPI="/dev/spidev0.0 ready"
else
    SPI="not available"
fi

# -- Forwarded ports (host → guest) --
# Host-side values are written by flake.nix to keep them in sync; guest ports
# are constants of the services they front. HTTP also serves gRPC-Web — it
# is multiplexed onto the same listener.
PORT_SSH='?'
PORT_HTTP='?'
# shellcheck source=/dev/null
[ -r /etc/bmc-virt/ports.env ] && . /etc/bmc-virt/ports.env
PORTS="SSH ${PORT_SSH}→22  HTTP/gRPC ${PORT_HTTP}→80  VNC 5900→5900"

# -- Print --
printf '\n'
printf '%s# BMC Virtual Machine%s\n' "$HEAD" "$RESET"
printf '%s|%s\n' \
    "Arch" "$ARCH ($ACCEL)" \
    "Kernel" "$KVER" \
    "DRM" "$DRM" \
    "WiFi" "$WIFI" \
    "SPI" "$SPI" \
    "Ports" "$PORTS" \
    | awk -F'|' -v d="$DIM" -v r="$RESET" '{ printf "%s%13s%s  %s\n", d, $1, r, $2 }'
printf '\n'

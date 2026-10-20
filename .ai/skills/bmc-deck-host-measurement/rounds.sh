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

# Usage: rounds.sh <device-ip> <label> <count>
# Restart the compositor <count> times on the Deck and append, per round,
# the host binary, idle RssAnon/RssFile and the raw startup-phase log lines
# to .tmp/deck-host-measurement/<label>-rounds.log.
set -euo pipefail

device=root@$1
label=$2
count=$3
out_dir=$(git rev-parse --show-toplevel)/.tmp/deck-host-measurement
mkdir -p "$out_dir"
out=$out_dir/$label-rounds.log

for round in $(seq 1 "$count"); do
    ssh "$device" sh -s -- "$label" "$round" <<'REMOTE' >>"$out"
label=$1
round=$2
f=/var/log/bmc/run-bmc-wasm-host-sdk-v0.log
t0=$(date -u +%Y-%m-%dT%H:%M:%S)
/etc/init.d/bmc-compositor restart
since() { awk -v t="$t0" '$1 > t' "$f"; }
i=0
until [ "$(since | grep -c 'wasm module loaded')" -ge 2 ]; do
    if since | grep -q 'slot rejected'; then echo "ROUND $label $round ABORT slot rejected"; exit 1; fi
    if [ $i -ge 180 ]; then echo "ROUND $label $round ABORT widgets did not load"; exit 1; fi
    sleep 1
    i=$((i + 1))
done
sleep 60
p=$(pidof bmc-wasm-host)
echo "ROUND $label $round t0=$t0 exe=$(readlink -f /proc/$p/exe) $(grep -E 'RssAnon|RssFile' /proc/$p/status | tr -s ' \t\n' ' ')"
since | grep -E 'starting bmc-wasm-host|Initializing GBM|EGL context created$|OpenGL ES:|FemtoVG screen target|renderer initialized|wasm module loaded|slot rejected'
REMOTE
done
echo "$out"

#!/usr/bin/env nix
#!nix shell nixpkgs#openssh nixpkgs#sshpass nixpkgs#ansifilter nixpkgs#coreutils nixpkgs#findutils
#!nix --command bash
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

# shellcheck shell=bash

# Pull all logs from the VM into vm-data/logs/, strip ANSI codes.

CALLER_CWD=$PWD

# shellcheck source=bmc-virt/scripts/_.sh
source "$(dirname "$0")/_.sh"

mkdir -p "$LOGDIR"

# Run on the guest, which defines these from its own paths.env — must not expand here.
# shellcheck disable=SC2016
LOG_SPECS=(
    'bmc.log:. /etc/bmc-virt/paths.env && cat "$BMC_LOG"'
    'relay.log:. /etc/bmc-virt/paths.env && cat "$RELAY_LOG"'
    'syslog.log:logread'
    'dmesg.log:dmesg'
)

for log_spec in "${LOG_SPECS[@]}"; do
    log_name=${log_spec%%:*}
    log_cmd=${log_spec#*:}
    ssh_vm "$log_cmd" >"$LOGDIR/$log_name"
done

# Strip ANSI escape codes from all pulled logs
find "$LOGDIR" \
    -name "*.log" \
    -exec sh -c 'tmp=$(mktemp) && ansifilter < "$1" > "$tmp" && mv "$tmp" "$1"' _ {} \;

echo "Logs saved to $LOGDIR/:"
for log_spec in "${LOG_SPECS[@]}"; do
    log_name=${log_spec%%:*}
    log_path=$LOGDIR/$log_name
    rel_path=$(realpath --relative-to="$CALLER_CWD" "$log_path")
    log_size_bytes=$(stat --printf='%s' "$log_path")
    log_size_human=$(numfmt --to=iec-i --suffix=B "$log_size_bytes")
    printf '  %s (%s)\n' "$rel_path" "$log_size_human"
done

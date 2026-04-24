#!/usr/bin/env nix
#!nix shell nixpkgs#openssh nixpkgs#sshpass nixpkgs#ansifilter nixpkgs#coreutils nixpkgs#findutils
#!nix --command bash
# shellcheck shell=bash

# Pull all logs from the VM into vm-data/logs/, strip ANSI codes.

CALLER_CWD=$PWD

# shellcheck source=bmc-virt/scripts/_.sh
source "$(dirname "$0")/_.sh"

mkdir -p "$LOGDIR"

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

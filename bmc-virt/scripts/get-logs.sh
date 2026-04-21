#!/usr/bin/env nix
#!nix shell nixpkgs#openssh nixpkgs#sshpass nixpkgs#ansifilter nixpkgs#coreutils nixpkgs#findutils
#!nix --command bash
# shellcheck shell=bash

# Pull all logs from the VM into vm-data/logs/, strip ANSI codes.

# shellcheck source=bmc-virt/scripts/_.sh
source "$(dirname "$0")/_.sh"

mkdir -p "$LOGDIR"
# Source paths from the VM so this can't drift from what the init scripts write.
ssh_vm '. /etc/bmc-virt/paths.env && cat "$BMC_LOG"' >"$LOGDIR/bmc.log"
ssh_vm '. /etc/bmc-virt/paths.env && cat "$RELAY_LOG"' >"$LOGDIR/relay.log"
ssh_vm "logread" >"$LOGDIR/syslog.log"
ssh_vm "dmesg" >"$LOGDIR/dmesg.log"

# Strip ANSI escape codes from all pulled logs
find "$LOGDIR" -name "*.log" -exec sh -c 'tmp=$(mktemp) && ansifilter < "$1" > "$tmp" && mv "$tmp" "$1"' _ {} \;

echo "Logs saved to $LOGDIR/"

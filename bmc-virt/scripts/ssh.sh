#!/usr/bin/env nix
#!nix shell nixpkgs#openssh nixpkgs#sshpass nixpkgs#coreutils
#!nix --command bash
# shellcheck shell=bash

# SSH into the running VM.

# shellcheck source=bmc-virt/scripts/_.sh
source "$(dirname "$0")/_.sh"
if [[ $# -eq 0 ]]; then
    ssh_vm -t bash -l
else
    ssh_vm "$@"
fi

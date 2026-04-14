#!/usr/bin/env nix
#!nix shell nixpkgs#openssh nixpkgs#sshpass nixpkgs#coreutils
#!nix --command bash
# shellcheck shell=bash

# SCP into the running VM.

# shellcheck source=bmc-virt/scripts/_.sh
source "$(dirname "$0")/_.sh"
scp_vm "$@"

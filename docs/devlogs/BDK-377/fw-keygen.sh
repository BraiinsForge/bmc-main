#!/usr/bin/env bash
# Generate a signify Ed25519 keypair compatible with OpenWrt usign.
#
# OpenWrt's usign is a fork of OpenBSD signify and accepts the same
# Ed25519 key/signature structures. Keys produced here can be used on
# the device by `usign -V` without conversion.
#
# Usage: fw-keygen [KEY_PREFIX]
#   KEY_PREFIX  default: bos-nix-init
#
# Outputs <prefix>.sec and <prefix>.pub in CWD. Refuses to overwrite.
set -euo pipefail

prefix="${1:-bos-nix-init}"
sec="${prefix}.sec"
pub="${prefix}.pub"

if [ -e "$sec" ] || [ -e "$pub" ]; then
    echo "error: $sec or $pub already exists — refusing to overwrite" >&2
    exit 1
fi

# -n: no passphrase (unattended CI signing). Drop -n to protect the key
# with a passphrase — manual signing will then prompt.
signify -G -n \
    -c "bos nix-init $(date -u +%Y-%m-%d)" \
    -s "$sec" -p "$pub"

chmod 0600 "$sec"
echo "wrote $sec $pub"

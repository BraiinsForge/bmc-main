#!/usr/bin/env bash
# Deploy a Nix-built package to the Deck.
# Copies the package + its full Nix closure to the device's /nix/store.
#
# Usage: ./scripts/nix-deploy.sh <package> [device-ip]
#
# Examples:
#   ./scripts/nix-deploy.sh bmc-openwrt-armv7-release 192.168.1.2
#   DEVICE_IP=192.168.1.2 ./scripts/nix-deploy.sh widgets-armv7-glibc-release
#
# Prerequisites on device:
#   - /nix bind mount exists (e.g. /mnt/data/nix -> /nix)
#   - SSH access as root
#   - nix-store available at /run/current-profile/bin/nix-store
set -euo pipefail

pkg="${1:?Usage: nix-deploy.sh <package> [device-ip]}"
device="${2:-${DEVICE_IP:?Set DEVICE_IP or pass as second argument}}"

store_path=$(nix build --no-link --print-out-paths ".#${pkg}^out")

echo "Copying closure to ${device}..."
nix copy --to "ssh://root@${device}?remote-program=/run/current-profile/bin/nix-store" "$store_path"
# shellcheck disable=SC2029 # Intentional client-side expansion of store_path
ssh "root@${device}" "/run/current-profile/bin/nix profile install ${store_path}"

echo "Done. Package at: ${store_path}"

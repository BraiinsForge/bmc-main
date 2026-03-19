#!/usr/bin/env bash
# Build a single widget with Nix and deploy it to the device.
# Copies the Nix closure to the device store, then symlinks the widget
# files into /run/current-profile file-by-file.
#
# Usage: ./scripts/nix-deploy-new-widget.sh <widget-name> [device-ip]
#
# Examples:
#   ./scripts/nix-deploy-new-widget.sh digital-clock 192.168.1.2
#   DEVICE_IP=192.168.1.2 ./scripts/nix-deploy-new-widget.sh flip-clock
#
# Prerequisites on device:
#   - /nix bind mount exists (e.g. /mnt/data/nix -> /nix)
#   - SSH access as root
#   - nix-store available at /run/current-profile/bin/nix-store
#   - the widget as a package output of the flake (widget-<name>-armv7-glibc-release)
set -euo pipefail

profile="/run/current-profile"

name="${1:?Usage: nix-deploy-new-widget.sh <widget-name> [device-ip]}"
device="${2:-${DEVICE_IP:?Set DEVICE_IP or pass as second argument}}"

pkg="widget-${name}-armv7-glibc-release"

echo "Building ${pkg}..."
store_path=$(nix build --no-link --print-out-paths ".#${pkg}^out")

echo "Copying closure to ${device}..."
nix copy --to "ssh://root@${device}?remote-program=/run/current-profile/bin/nix-store" "$store_path"

echo "Symlinking widget files into ${profile}..."
# shellcheck disable=SC2029 # Intentional client-side expansion
ssh "root@${device}" "
    cd '${store_path}'
    find . -type f -o -type l | while read -r file; do
        dir=\$(dirname \"\$file\")
        mkdir -p '${profile}/'\"\$dir\"
        ln -sf '${store_path}/'\"\$file\" '${profile}/'\"\$file\"
    done
"

echo "Done. Widget '${name}' deployed from ${store_path}."

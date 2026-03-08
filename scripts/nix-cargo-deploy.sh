#!/usr/bin/env bash
# Deploy a cargo-built binary to the device, replacing it in-place.
# This is for fast iteration — no Nix rebuild, just scp the binary.
#
# Usage: ./scripts/nix-cargo-deploy.sh <command> [args...] [device-ip]
#
# Commands:
#   compositor [device-ip]        - deploy bmc-openwrt binary
#   widget <name> [device-ip]     - deploy a widget by name (e.g. digital-clock)
#
# Examples:
#   ./scripts/nix-cargo-deploy.sh compositor 192.168.1.2
#   ./scripts/nix-cargo-deploy.sh widget digital-clock 192.168.1.2
#   DEVICE_IP=192.168.1.2 ./scripts/nix-cargo-deploy.sh widget flip-clock
#
# Prerequisites:
#   - Binary already built with cargo (in the appropriate nix develop shell)
#   - Device initialized with nix-init.sh and packages deployed with nix-deploy.sh
set -euo pipefail

profile="/run/current-profile"

cmd="${1:?Usage: nix-cargo-deploy.sh <compositor|widget> ...}"
shift
label="$cmd"

case "$cmd" in
compositor)
    device="${1:-${DEVICE_IP:?Set DEVICE_IP or pass as argument}}"
    local_bin="target/armv7-unknown-linux-gnueabihf/release/bmc-openwrt"
    remote_path="${profile}/bin/bmc-openwrt"
    ;;
widget)
    name="${1:?Usage: nix-cargo-deploy.sh widget <name> [device-ip]}"
    shift
    device="${1:-${DEVICE_IP:?Set DEVICE_IP or pass as argument}}"
    local_bin="target/armv7-unknown-linux-gnueabihf/release/bmc-widget-${name}"
    remote_path="${profile}/lib/bmc-widgets/${name}/bin/bmc-widget-${name}"
    label="widget ${name}"
    ;;
*)
    echo "Unknown command: ${cmd}"
    echo "Usage: nix-cargo-deploy.sh <compositor|widget> ..."
    exit 1
    ;;
esac

if [ ! -f "$local_bin" ]; then
    echo "Error: ${local_bin} not found."
    echo "Build it first (in the appropriate nix develop shell)."
    exit 1
fi

echo "Deploying ${label} to ${device}..."
scp "$local_bin" "root@${device}:${remote_path}"
echo "Done."

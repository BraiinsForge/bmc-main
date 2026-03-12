#!/usr/bin/env bash
# Deploy a cargo-built binary to the device for fast iteration.
# The binary is uploaded to a staging directory and the profile entry
# is symlinked to point to it.
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
deploy_dir="/mnt/data/tmp/cargo-deploy"

cmd="${1:?Usage: nix-cargo-deploy.sh <compositor|widget> ...}"
shift
label="$cmd"

case "$cmd" in
compositor)
    device="${1:-${DEVICE_IP:?Set DEVICE_IP or pass as argument}}"
    local_bin="target/armv7-unknown-linux-gnueabihf/release/bmc-openwrt"
    bin_name="bmc-openwrt"
    profile_path="${profile}/bin/bmc-openwrt"
    ;;
widget)
    name="${1:?Usage: nix-cargo-deploy.sh widget <name> [device-ip]}"
    shift
    device="${1:-${DEVICE_IP:?Set DEVICE_IP or pass as argument}}"
    local_bin="target/armv7-unknown-linux-gnueabihf/release/bmc-widget-${name}"
    bin_name="bmc-widget-${name}"
    profile_path="${profile}/lib/bmc-widgets/${name}/bin/bmc-widget-${name}"
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

deploy_path="${deploy_dir}/${bin_name}"

echo "Deploying ${label} to ${device}..."

# Back up the original binary (only on first deploy)
# shellcheck disable=SC2029 # Intentional client-side expansion
ssh "root@${device}" "[ -f ${profile_path}.orig ] || cp ${profile_path} ${profile_path}.orig"

# Upload binary to staging directory
# shellcheck disable=SC2029 # Intentional client-side expansion
ssh "root@${device}" "mkdir -p ${deploy_dir}"
scp "$local_bin" "root@${device}:${deploy_path}"

# Symlink profile entry to the deployed binary
# shellcheck disable=SC2029 # Intentional client-side expansion
ssh "root@${device}" "ln -sf ${deploy_path} ${profile_path}"

echo "Done. Binary at ${deploy_path}, symlinked from ${profile_path}."

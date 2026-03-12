#!/usr/bin/env bash
# Deploy a cargo-built binary to the device without overwriting Nix wrappers.
# The binary is uploaded to a staging directory and the wrapper script's exec
# line is patched to point to the new binary.
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
deploy_dir="/mnt/data/tmp/cargo-deploy"

cmd="${1:?Usage: nix-cargo-deploy.sh <compositor|widget> ...}"
shift
label="$cmd"

case "$cmd" in
compositor)
    device="${1:-${DEVICE_IP:?Set DEVICE_IP or pass as argument}}"
    local_bin="target/armv7-unknown-linux-gnueabihf/release/bmc-openwrt"
    bin_name="bmc-openwrt"
    wrapper_path="${profile}/bin/bmc-openwrt"
    ;;
widget)
    name="${1:?Usage: nix-cargo-deploy.sh widget <name> [device-ip]}"
    shift
    device="${1:-${DEVICE_IP:?Set DEVICE_IP or pass as argument}}"
    local_bin="target/armv7-unknown-linux-gnueabihf/release/bmc-widget-${name}"
    bin_name="bmc-widget-${name}"
    wrapper_path="${profile}/lib/bmc-widgets/${name}/bin/bmc-widget-${name}"
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

# Back up the original wrapper (only on first deploy)
# shellcheck disable=SC2029 # Intentional client-side expansion of wrapper_path
ssh "root@${device}" "[ -f ${wrapper_path}.orig ] || cp ${wrapper_path} ${wrapper_path}.orig"

# Upload binary to staging directory (preserves the wrapper)
# shellcheck disable=SC2029 # Intentional client-side expansion of deploy_dir
ssh "root@${device}" "mkdir -p ${deploy_dir}"
scp "$local_bin" "root@${device}:${deploy_path}"

# Patch the wrapper's last exec line to point to the deployed binary
# shellcheck disable=SC2029 # Intentional client-side expansion of deploy_path/wrapper_path
ssh "root@${device}" "sed -i '\$s|^exec .*|exec ${deploy_path} \"\\\$@\"|' ${wrapper_path}"

echo "Done. Binary at ${deploy_path}, wrapper at ${wrapper_path} patched."

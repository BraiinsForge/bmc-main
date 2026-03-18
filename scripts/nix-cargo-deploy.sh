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
#   - Executed in a devshell, such as .#armv7-glibc-release
#   - Device initialized with nix-init.sh and packages deployed with nix-deploy.sh
# Extra cargo flags:
#   If you need to add extra flags, such as `--features profiling` for the compositor,
#   use the environment variable CARGO_EXTRA_FLAGS.
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

echo "Building..."
set -x
# shellcheck disable=SC2086 # Intentional word splitting on CARGO_EXTRA_FLAGS
cargo build --release --target armv7-unknown-linux-gnueabihf -p "$bin_name" ${CARGO_EXTRA_FLAGS:-}
set +x

deploy_path="${deploy_dir}/${bin_name}"

# Verify the target path exists on the device (requires prior nix-deploy.sh)
# shellcheck disable=SC2029 # Intentional client-side expansion
if ! ssh "root@${device}" "[ -e ${profile_path} ]"; then
    echo "Error: ${profile_path} not found on ${device}."
    echo "Deploy the full package first with nix-deploy.sh."
    exit 1
fi

echo "Deploying ${label} to ${device}..."

# Copy nix store paths (dynamic linker + rpath libraries) to the device
interp=$(patchelf --print-interpreter "$local_bin")
rpath=$(patchelf --print-rpath "$local_bin" | tr ':' '\n' | grep '^/nix/store/' | tr '\n' ' ')
# shellcheck disable=SC2086 # Intentional word splitting on space-separated store paths
nix copy --to "ssh://root@${device}?remote-program=/run/current-profile/bin/nix-store" \
    "$interp" $rpath

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

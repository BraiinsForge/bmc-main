#!/usr/bin/env bash
# Deploy and run orchestrator integration tests on a device.
# Usage: DEVICE_IP=192.168.x.x ./deploy.sh
set -euo pipefail

DEVICE_IP="${DEVICE_IP:?Set DEVICE_IP to the device address}"

echo "Building test-orchestrator..."
store_path=$(nix build --no-link --print-out-paths .#default)

echo "Copying to ${DEVICE_IP}..."
nix copy --to "ssh://root@${DEVICE_IP}?remote-program=/run/current-profile/bin/nix-store" "$store_path"

echo "Running tests on ${DEVICE_IP}..."
# shellcheck disable=SC2029 # Intentional client-side expansion of store_path
ssh "root@${DEVICE_IP}" "${store_path}/bin/test-orchestrator"

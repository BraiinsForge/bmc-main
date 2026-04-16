#!/usr/bin/env bash
# Ensure bmc-nix-cli is available on the target device.
#
# If already present in /run/current-profile, prints that path.
# Otherwise builds the ARM package locally, copies its closure
# to the device, registers it in the bmc profile via its own
# add-packages command (one-time bootstrap), and prints the
# profile path to the binary.
#
# Usage: ./scripts/ensure-bmc-nix-cli.sh <device-ip>
#
# Prints the full path to bmc-nix-cli on the device to stdout.
# All other output goes to stderr.
set -euo pipefail

device="${1:?Usage: ensure-bmc-nix-cli.sh <device-ip>}"

readonly PROFILE_CLI="/run/current-profile/bin/bmc-nix-cli"
readonly FLAKE_ATTR=".#deck-packages.bmc-nix-cli"
readonly REMOTE_NIX_STORE="/run/current-profile/bin/nix-store"
readonly PROFILE_DIR="/nix/var/nix/gcroots/profiles/bmc"

# Check if bmc-nix-cli is already on the device
# shellcheck disable=SC2029 # Intentional client-side expansion
if ssh "root@${device}" "test -x ${PROFILE_CLI}" 2>/dev/null; then
    echo >&2 "bmc-nix-cli already available at ${PROFILE_CLI}"
    echo "${PROFILE_CLI}"
    exit 0
fi

echo >&2 "bmc-nix-cli not found on device, bootstrapping..."

# Build the ARM package locally
store_path=$(nix build --no-link --print-out-paths "${FLAKE_ATTR}.pkg")
version=$(nix eval --raw "${FLAKE_ATTR}.version")
echo >&2 "Built bmc-nix-cli ${version}: ${store_path}"

# Copy closure to device (only populates /nix/store)
echo >&2 "Copying closure to ${device}..."
nix copy --to "ssh://root@${device}?remote-program=${REMOTE_NIX_STORE}" "${store_path}"

# Self-install: run the just-copied bmc-nix-cli to register itself in the
# bmc profile, so future invocations find it at ${PROFILE_CLI}.
echo >&2 "Registering bmc-nix-cli in ${PROFILE_DIR}..."
# shellcheck disable=SC2029 # Intentional client-side expansion
ssh "root@${device}" \
    "PATH=/run/current-profile/bin:\$PATH \
     ${store_path}/bin/bmc-nix-cli add-packages \
        --profile-dir ${PROFILE_DIR} \
        --name bmc-nix-cli --version '${version}' --store-path '${store_path}'"

echo >&2 "bmc-nix-cli bootstrapped at ${PROFILE_CLI}"
echo "${PROFILE_CLI}"

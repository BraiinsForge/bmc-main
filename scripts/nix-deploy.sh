#!/usr/bin/env bash
# Deploy a package to the device's bmc profile.
# Builds the package, copies its Nix closure to the device, then
# runs bmc-nix-cli to add it to the profile.
#
# Usage: ./scripts/nix-deploy.sh <flake-uri> [device-ip]
#
# The first argument is a full flake URI:
#   .#deck-packages.<name>   — index package (has .pkg, .version)
#   .#armv7-nixpkgs.<name>    — raw nixpkgs derivation (has .pname, .version)
#   nixpkgs#<name>            — upstream nixpkgs derivation
#
# Examples:
#   ./scripts/nix-deploy.sh '.#deck-packages.core' 192.168.1.2
#   ./scripts/nix-deploy.sh '.#deck-packages.digital-clock' 192.168.1.2
#   ./scripts/nix-deploy.sh '.#armv7-nixpkgs.bash' 192.168.1.2
#   ./scripts/nix-deploy.sh 'nixpkgs#hello' 192.168.1.2
#   DEVICE_IP=192.168.1.2 ./scripts/nix-deploy.sh '.#deck-packages.flip-clock'
#
# Environment variables:
#   DEVICE_IP     — default device IP (overridden by second argument)
#   NIX_OUTPUT    — derivation output to build (default: out)
#
# Prerequisites on device:
#   - /nix bind mount exists (e.g. /mnt/data/nix -> /nix)
#   - SSH access as root
#   - nix-store available at /run/current-profile/bin/nix-store
set -euxo pipefail

attr="${1:?Usage: nix-deploy.sh <flake-uri> [device-ip]}"
device="${2:-${DEVICE_IP:?Set DEVICE_IP or pass as second argument}}"
output="${NIX_OUTPUT:-out}"

# Ensure bmc-nix-cli is available on the device (bootstraps if needed)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bmc_nix_cli=$("${SCRIPT_DIR}/ensure-bmc-nix-cli.sh" "${device}")

# Auto-detect: index package (.pkg exists) vs raw nixpkgs derivation
if nix eval "${attr}.pkg.name" &>/dev/null; then
    # Index package — build .pkg, read .version, use attr leaf as name
    echo "Building index package ${attr}..."
    store_path=$(nix build --no-link --print-out-paths "${attr}.pkg^${output}")
    version=$(nix eval --raw "${attr}.version")
    deploy_name="${attr##*.}"
else
    # Raw nixpkgs derivation — build directly, prefix with nixpkgs-
    echo "Building nixpkgs package ${attr}..."
    store_path=$(nix build --no-link --print-out-paths "${attr}^${output}")
    version=$(nix eval --raw "${attr}.version")
    deploy_name="nixpkgs-$(nix eval --raw "${attr}.pname")"
fi

echo "Copying closure to ${device}..."
nix copy --to "ssh://root@${device}?remote-program=/run/current-profile/bin/nix-store" "$store_path"

# shellcheck disable=SC2029 # Intentional client-side expansion
ssh "root@${device}" \
    "PATH=/run/current-profile/bin:\$PATH \
     ${bmc_nix_cli} add-packages \
        --profile-dir /nix/var/nix/gcroots/profiles/bmc \
        --name '${deploy_name}' --version '${version}' --store-path '${store_path}'"

echo "Done. Package '${deploy_name}' v${version} deployed at: ${store_path}"

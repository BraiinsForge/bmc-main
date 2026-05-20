#!/usr/bin/env bash
# Deploy packages to the device's bmc profile.
# Builds the packages, copies their Nix closures to the device, then
# runs bmc-nix-cli once to add them to the profile.
#
# Usage: ./scripts/nix-deploy.sh <flake-uri>... [device-ip]
#
# Package arguments are full flake URIs:
#   .#deck-packages.<name>   — index package (has .pkg, .version)
#   .#armv7-nixpkgs.<name>    — raw nixpkgs derivation (has .pname, .version)
#
# Examples:
#   ./scripts/nix-deploy.sh '.#deck-packages.core' 192.168.1.2
#   ./scripts/nix-deploy.sh '.#deck-packages.core' '.#deck-packages.pomodoro' 192.168.1.2
#   ./scripts/nix-deploy.sh '.#deck-packages.digital-clock' 192.168.1.2
#   ./scripts/nix-deploy.sh '.#armv7-nixpkgs.bash' 192.168.1.2
#   DEVICE_IP=192.168.1.2 ./scripts/nix-deploy.sh '.#deck-packages.flip-clock'
#
# Environment variables:
#   DEVICE_IP     — default device IP (overridden by trailing IPv4 argument)
#   NIX_OUTPUT    — derivation output to build (default: out)
#
# Prerequisites on device:
#   - /nix bind mount exists (e.g. /mnt/data/nix -> /nix)
#   - SSH access as root
#   - nix-store available at /run/current-profile/bin/nix-store
set -euxo pipefail

usage="Usage: nix-deploy.sh <flake-uri>... [device-ip]"

is_ipv4() {
    [[ $1 =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}$ ]]
}

if [ "$#" -eq 0 ]; then
    echo "${usage}" >&2
    exit 1
fi

attrs=("$@")
last_index=$((${#attrs[@]} - 1))
last_arg="${attrs[${last_index}]}"

if is_ipv4 "${last_arg}"; then
    device="${last_arg}"
    unset "attrs[${last_index}]"
else
    device="${DEVICE_IP:?Set DEVICE_IP or pass a trailing IPv4 address}"
fi

if [ "${#attrs[@]}" -eq 0 ]; then
    echo "${usage}" >&2
    exit 1
fi

output="${NIX_OUTPUT:-out}"

# Ensure bmc-nix-cli is available on the device (bootstraps if needed)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bmc_nix_cli=$("${SCRIPT_DIR}/ensure-bmc-nix-cli.sh" "${device}")

build_installables=()
deploy_names=()
versions=()

for attr in "${attrs[@]}"; do
    # Auto-detect: index package (.pkg exists) vs raw nixpkgs derivation.
    if nix eval "${attr}.pkg.name" &>/dev/null; then
        # Index package — build .pkg, read .version, use attr leaf as name.
        echo "Queueing index package ${attr}..."
        build_installables+=("${attr}.pkg^${output}")
        versions+=("$(nix eval --raw "${attr}.version")")
        deploy_names+=("${attr##*.}")
    else
        # Raw nixpkgs derivation — build directly, prefix with nixpkgs-.
        echo "Queueing nixpkgs package ${attr}..."
        build_installables+=("${attr}^${output}")
        versions+=("$(nix eval --raw "${attr}.version")")
        deploy_names+=("nixpkgs-$(nix eval --raw "${attr}.pname")")
    fi
done

echo "Building ${#build_installables[@]} package(s)..."
build_output=$(nix build --no-link --print-out-paths "${build_installables[@]}")
mapfile -t store_paths <<<"${build_output}"

if [ "${#store_paths[@]}" -ne "${#attrs[@]}" ]; then
    echo "Expected ${#attrs[@]} store path(s), got ${#store_paths[@]}" >&2
    exit 1
fi

echo "Copying closures to ${device}..."
nix copy --to "ssh://root@${device}?remote-program=/run/current-profile/bin/nix-store" "${store_paths[@]}"

cli_args=(
    add-packages
    --profile-dir /nix/var/nix/gcroots/profiles/bmc
)

for i in "${!store_paths[@]}"; do
    cli_args+=(
        --name "${deploy_names[${i}]}"
        --version "${versions[${i}]}"
        --store-path "${store_paths[${i}]}"
    )
done

printf -v remote_cli '%q ' "${bmc_nix_cli}" "${cli_args[@]}"

# shellcheck disable=SC2029 # Intentional client-side expansion
ssh "root@${device}" \
    "PATH=/run/current-profile/bin:\$PATH ${remote_cli}"

echo "Done. Deployed ${#store_paths[@]} package(s):"
for i in "${!store_paths[@]}"; do
    echo "  ${deploy_names[${i}]} v${versions[${i}]} at: ${store_paths[${i}]}"
done

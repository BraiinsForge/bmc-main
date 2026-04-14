#!/usr/bin/env bash

# Build everything, start VM if needed, deploy, connect.
#
# Usage: ./scripts/run.sh [options]
#   --rr              Start bmc-openwrt under rr (time-travel debugger)
#   --config <path>   Deploy a specific config file
#   --profile <name>  Build profile (default: auto-detected from host arch)
#   --host-path <dirs> Colon-separated dirs to add to VM PATH
#
# Shorthand configs:
#   --customer        Use data/bmc_config_customer.json
#   --frantisek       Use data/bmc_config_frantisek.json

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

"$SCRIPT_DIR/stop.sh" || true

while [[ $# -gt 0 ]]; do
    case "$1" in
    --rr)
        if [[ "$(uname -m)" == "arm64" || "$(uname -m)" == "aarch64" ]]; then
            echo "ERROR: rr is x86_64-only, not available on aarch64 hosts" >&2
            exit 1
        fi
        export RR=1
        export BMC_PROFILE=x86_64-rr
        shift
        ;;
    --config)
        CONFIG="$(realpath "$2")"
        export CONFIG
        shift 2
        ;;
    --profile)
        export BMC_PROFILE="$2"
        shift 2
        ;;
    --host-path)
        export BMC_VIRT_HOST_PATH="$2"
        shift 2
        ;;
    --customer)
        CONFIG="$(realpath data/bmc_config_customer.json)"
        export CONFIG
        shift
        ;;
    --frantisek)
        CONFIG="$(realpath data/bmc_config_frantisek.json)"
        export CONFIG
        shift
        ;;
    *)
        echo "Unknown option: $1" >&2
        exit 1
        ;;
    esac
done

# Ensure platform-specific build prerequisites before any nix builds.
"$SCRIPT_DIR/darwin-ensure-builder.sh"
"$SCRIPT_DIR/linux-ensure-binfmt.sh"

nix run -L ".#run"

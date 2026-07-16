#!/usr/bin/env bash
# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

# Build everything, start VM if needed, deploy, connect.
#
# Usage: ./scripts/run.sh [options]
#   --rr              Start bmc-openwrt under rr (time-travel debugger)
#   --config <name>   Deploy data/configs/<name>.json
#   --profile <name>  Build profile (default: auto-detected from host arch)
#   --host-path <dirs> Colon-separated dirs to add to VM PATH

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

CONFIG_DIR="$PWD/data/configs"

resolve_config() {
    local name="$1"
    local config_path="$CONFIG_DIR/$name.json"

    if [[ $name == */* || $name == *.json ]]; then
        echo "ERROR: --config expects a bare config name from data/configs, without path or extension" >&2
        exit 1
    fi

    if [[ ! -f $config_path ]]; then
        echo "ERROR: config '$name' not found at $config_path" >&2
        exit 1
    fi

    realpath "$config_path"
}

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
        CONFIG="$(resolve_config "$2")"
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

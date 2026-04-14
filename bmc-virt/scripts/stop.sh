#!/usr/bin/env bash

# Kill QEMU process.

set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/.."
nix run -L ".#stop"

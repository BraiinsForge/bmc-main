#!/usr/bin/env bash

# Open display window for a running VM (Xvfb → ffplay with rotation).

set -euo pipefail
cd "$(cd "$(dirname "$0")" && pwd)/.."
nix run ".#display"

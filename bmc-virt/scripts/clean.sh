#!/usr/bin/env bash

# Remove all runtime data (overlay image, logs, cached hashes).

set -euo pipefail
cd "$(cd "$(dirname "$0")" && pwd)/.."
git clean -fdx vm-data/

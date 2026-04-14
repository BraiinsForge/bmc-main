#!/usr/bin/env bash

# Generate a mermaid crate dependency graph from the workspace Cargo.toml files.
# Usage: ./gen-crate-deps.sh > crate-deps.mermaid
#
# Requires: cargo (for metadata)

set -euo pipefail

WORKSPACE_ROOT="${1:-$(git rev-parse --show-toplevel)}"

echo "graph TD"

cargo metadata --manifest-path "$WORKSPACE_ROOT/Cargo.toml" --format-version 1 --no-deps 2>/dev/null \
    | jq -r '
    .packages[] |
    .name as $name |
    .dependencies[] |
    select(.path != null) |
    "  \($name | gsub("-";"_")) --> \(.name | gsub("-";"_"))"
  ' | sort -u

#!/usr/bin/env bash
set -euo pipefail

# CONFIG
UPSTREAM_REPO="ssh://git@gitlab.ii.zone/tooling/tooling.git"
UPSTREAM_COMMIT="0bf48952586c5e475368fad41d05b4ddb2b6a079"

# ARGS
if [ $# -ne 1 ]; then
    echo "Usage: $0 <path_to_vendored_subtree>"
    exit 1
fi

VENDORED_ROOT="$(realpath "$1")"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# Clone upstream repo at specific commit
echo "📥 Cloning upstream repo..."
git clone --quiet "$UPSTREAM_REPO" "$TMP_DIR"
cd "$TMP_DIR"
git checkout --quiet "$UPSTREAM_COMMIT"

# Go back to original dir
cd -

# Find all local crates (by presence of Cargo.toml)
echo "🔍 Finding vendored crates under $VENDORED_ROOT..."
mapfile -t RELATIVE_CRATES < <(find "$VENDORED_ROOT" -name Cargo.toml -exec dirname {} \; | sed "s|$VENDORED_ROOT/||")

# Define hash function
hash_dir() {
    local dir="$1"
    find "$dir" -type f ! -path "*/target/*" -exec sha256sum {} \; | sed "s|$dir/||" | sort
}

# Compare each crate
all_match=true
for crate in "${RELATIVE_CRATES[@]}"; do
    echo "🔄 Checking crate: $crate"

    upstream_crate_path="$TMP_DIR/$crate"
    local_crate_path="$VENDORED_ROOT/$crate"

    if [ ! -d "$upstream_crate_path" ]; then
        echo "⚠️  Skipping: '$crate' does not exist in upstream."
        continue
    fi

    hash_dir "$upstream_crate_path" >"$TMP_DIR/upstream.hashes"
    hash_dir "$local_crate_path" >"$TMP_DIR/local.hashes"

    if diff -q "$TMP_DIR/upstream.hashes" "$TMP_DIR/local.hashes"; then
        echo "✅ $crate matches upstream."
    else
        echo "❌ $crate differs from upstream!"
        diff "$TMP_DIR/upstream.hashes" "$TMP_DIR/local.hashes" | head -n 20
        all_match=false
    fi
done

if ! $all_match; then
    echo "❌ One or more crates are out of sync with upstream."
    exit 1
else
    echo "🎉 All vendored crates match upstream commit $UPSTREAM_COMMIT"
fi

#!/usr/bin/env bash
# Verify all bmc-virt kernel patches apply cleanly against the actual kernel
# source + OpenWrt patches. Fast — no compilation, just patch --dry-run.
#
# Usage: ./scripts/check-patches.sh
#
# Downloads the kernel source on first run (~130MB), caches in /tmp.
# Subsequent runs reuse the cache and finish in seconds.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VIRT_DIR="$SCRIPT_DIR/.."
PATCHES_DIR="$VIRT_DIR/kernel-patches"

# Read version from flake.nix
LINUX_VERSION=$(grep 'linuxVersion = ' "$VIRT_DIR/flake.nix" | head -1 | sed 's/.*"\(.*\)".*/\1/')
CACHE_DIR="/tmp/bmc-virt-patch-check/linux-${LINUX_VERSION}"

if [[ ! -d $CACHE_DIR ]]; then
    echo "Downloading linux-${LINUX_VERSION} source..."
    mkdir -p "$(dirname "$CACHE_DIR")"
    curl -sL "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${LINUX_VERSION}.tar.xz" \
        | tar xJ -C "$(dirname "$CACHE_DIR")"
    echo "Cached at $CACHE_DIR"
fi

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

echo "Preparing patched source..."
cp -a "$CACHE_DIR/net" "$WORKDIR/net"
cp -a "$CACHE_DIR/drivers" "$WORKDIR/drivers"
# Copy only dirs our patches touch — fast

# Apply OpenWrt patches that affect our patched files (silent, non-fatal)
OPENWRT_SRC=$(nix eval --raw "path:$VIRT_DIR#openwrtSrc" 2>/dev/null || echo "")
if [[ -n $OPENWRT_SRC && -d $OPENWRT_SRC ]]; then
    for patchdir in \
        "$OPENWRT_SRC/target/linux/generic/backport-6.6" \
        "$OPENWRT_SRC/target/linux/generic/pending-6.6" \
        "$OPENWRT_SRC/target/linux/generic/hack-6.6"; do
        if [[ -d $patchdir ]]; then
            for p in "$patchdir"/*.patch; do
                patch -d "$WORKDIR" -p1 -s <"$p" 2>/dev/null || true
            done
        fi
    done
    echo "OpenWrt patches applied."
else
    echo "WARNING: Could not resolve openwrtSrc, testing against vanilla kernel only."
    echo "         Patches may succeed here but fail in the full build if OpenWrt"
    echo "         patches shifted line numbers. Run with 'nix develop' for full check."
fi

# Our patches MUST apply cleanly
echo ""
echo "=== Verifying bmc-virt kernel patches ==="
FAILED=0
for p in "$PATCHES_DIR"/*.patch; do
    name=$(basename "$p")
    if patch -d "$WORKDIR" -p1 --dry-run -s <"$p" 2>/dev/null; then
        echo "  ✓ $name"
    else
        echo "  ✗ $name FAILED"
        echo "    Re-running with verbose output:"
        patch -d "$WORKDIR" -p1 --dry-run <"$p" || true
        FAILED=1
    fi
done

echo ""
if [[ $FAILED -eq 0 ]]; then
    echo "All patches apply cleanly."
else
    echo "SOME PATCHES FAILED — see above."
    exit 1
fi

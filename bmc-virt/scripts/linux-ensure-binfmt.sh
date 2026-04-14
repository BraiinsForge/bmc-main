#!/usr/bin/env bash
# Ensure x86_64 binfmt emulation is available (aarch64-linux only).
# The OpenWrt ImageBuilder is always an x86_64-linux binary. On aarch64 hosts,
# Nix needs qemu-user-static binfmt + extra-platforms to run it.
#
# This is the Linux counterpart to darwin-ensure-builder.sh.

set -euo pipefail

# Not Linux → nothing to do (macOS has its own builder script).
if [[ "$(uname -s)" != "Linux" ]]; then
    exit 0
fi

# x86_64 hosts run the ImageBuilder natively → nothing to do.
if [[ "$(uname -m)" == "x86_64" ]]; then
    exit 0
fi

ERRORS=()

# 1. Check that qemu-user-static provides the x86_64 binfmt handler.
if [[ ! -f /proc/sys/fs/binfmt_misc/qemu-x86_64 ]]; then
    ERRORS+=("x86_64 binfmt handler not registered (/proc/sys/fs/binfmt_misc/qemu-x86_64 missing)")
fi

# 2. Check that Nix is configured to use it.
NIX_CONF_DIRS=(/etc/nix/nix.conf /etc/nix/nix.custom.conf)
FOUND_EXTRA_PLATFORMS=false
for conf in "${NIX_CONF_DIRS[@]}"; do
    if [[ -f $conf ]] && grep -qE '^\s*extra-platforms\s*=.*x86_64-linux' "$conf" 2>/dev/null; then
        FOUND_EXTRA_PLATFORMS=true
        break
    fi
done
if [[ $FOUND_EXTRA_PLATFORMS == "false" ]]; then
    ERRORS+=("Nix is not configured with extra-platforms = x86_64-linux")
fi

if [[ ${#ERRORS[@]} -eq 0 ]]; then
    echo "x86_64 binfmt emulation ready."
    exit 0
fi

echo "ERROR: x86_64 binfmt emulation is not set up." >&2
echo "" >&2
for err in "${ERRORS[@]}"; do
    echo "  - $err" >&2
done
echo "" >&2
echo "To fix, run:" >&2
echo "" >&2
echo "  sudo apt install qemu-user-static binfmt-support" >&2
echo '  echo "extra-platforms = x86_64-linux" | sudo tee -a /etc/nix/nix.custom.conf' >&2
echo "  sudo systemctl restart nix-daemon" >&2
echo "" >&2
exit 1

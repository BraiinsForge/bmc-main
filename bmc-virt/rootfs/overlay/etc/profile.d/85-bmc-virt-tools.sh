#!/bin/sh

# Discovery aid for the interactive login banner.
#
# Lists which of the non-busybox debugging tools we ship in this image
# are actually present, checked at shell-init time via `command -v`.
#
# The name list intentionally tracks `bmc-virt/dl-cache/openwrt-config.nix`.
# Add a package there and add the binary's name here;
# If the package gets dropped the entry is silently elided
# rather than turning into a phantom hint.
#
# Doubles as a cheat sheet for any LLM that SSHes in and reads
# the banner before deciding what to invoke.

[ -t 1 ] || return 0
[ -n "${BMC_VIRT_TOOLS_SHOWN:-}" ] && return 0
export BMC_VIRT_TOOLS_SHOWN=1

ESC=$(printf '\033')
HEAD="${ESC}[33;1m"
DIM="${ESC}[2m"
RESET="${ESC}[0m"

# /usr/bin entries are either busybox symlinks (base image) or real binaries
# installed by opkg packages. The non-symlinks are everything we layered on
# top: curl, jq, socat, strace, xxd, od, plus utility upgrades like
# ip-full's `ip`. Listing them lets the banner self-update when packages
# are added or dropped from openwrt-config.nix — no parallel name list.
present=$(
    for p in /usr/bin/*; do
        [ -L "$p" ] && continue
        [ -x "$p" ] || continue
        printf '%s\n' "${p##*/}"
    done | sort | tr '\n' ' '
)

[ -z "$present" ] && return 0

printf '\n'
printf '%s# Debug tools%s  %s%s%s\n' "$HEAD" "$RESET" "$DIM" "$present" "$RESET"
printf '\n'

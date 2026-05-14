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
    done | sort
)

[ -z "$present" ] && return 0

printf '\n'
printf '%s# Available tools%s\n' "$HEAD" "$RESET"
echo "$present" | awk -v cols=4 -v d="$DIM" -v r="$RESET" '
    { rows[NR] = $0; if (length > w) w = length }
    END {
        w += 2
        for (i = 1; i <= NR; i++) {
            # Hand-pad rather than `%*s`; busybox awk does not support the
            # dynamic-width specifier and fails the whole call with
            # "%*x formats are not supported".
            cell = rows[i]
            while (length(cell) < w) cell = cell " "
            printf "%s%s%s", d, cell, r
            if (i % cols == 0) printf "\n"
        }
        if (NR % cols != 0) printf "\n"
    }
'
printf '\n'

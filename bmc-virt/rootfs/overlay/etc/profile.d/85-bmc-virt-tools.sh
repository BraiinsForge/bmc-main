#!/bin/sh
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

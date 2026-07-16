#!/usr/bin/env bash
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

# Run a command inside a memory-capped systemd cgroup scope so a runaway
# cargo/clippy/rustc process can't OOM the shell.
#
# Usage: scripts/mem-box.sh <cmd> [args...]
#
# On Linux with systemd, wraps the command in a transient user scope whose
# MemoryMax is 80 % of currently-available memory (soft-throttle at 70 %).
# On macOS / non-systemd Linux, falls through to plain exec — this script
# is a no-op there.
set -eu

if [ "$#" -eq 0 ]; then
    echo "usage: $0 <cmd> [args...]" >&2
    exit 2
fi

# Format bytes as "N.N GiB" / "N.N MiB".
fmt_bytes() {
    numfmt --to=iec-i --suffix=B --format="%.1f" "$1"
}

# ANSI 256-color orange, disabled if stderr isn't a TTY or NO_COLOR is set.
if [ -t 2 ] && [ -z "${NO_COLOR:-}" ]; then
    c_on=$'\033[38;5;208m'
    c_off=$'\033[0m'
else
    c_on=""
    c_off=""
fi

# Print a unicode box to stderr. Args: <title> <line1> <line2> ...
# Box glyphs and title are colored; content lines are plain.
render_box() {
    local title="$1"
    shift
    local lines=("$@")
    local w=${#title}
    local l
    for l in "${lines[@]}"; do
        [ "${#l}" -gt "$w" ] && w=${#l}
    done
    local hline
    hline=$(printf '─%.0s' $(seq $((w + 2))))
    {
        echo ""
        printf '%s╭%s╮%s\n' "$c_on" "$hline" "$c_off"
        printf '%s│ %s%-*s%s │%s\n' "$c_on" "$c_off" "$w" "$title" "$c_on" "$c_off"
        printf '%s├%s┤%s\n' "$c_on" "$hline" "$c_off"
        for l in "${lines[@]}"; do
            printf '%s│%s %-*s %s│%s\n' "$c_on" "$c_off" "$w" "$l" "$c_on" "$c_off"
        done
        printf '%s╰%s╯%s\n' "$c_on" "$hline" "$c_off"
        echo ""
    } >&2
}

if command -v systemd-run >/dev/null 2>&1 && [ -r /proc/meminfo ]; then
    avail=$(awk '/^MemAvailable:/ {print $2 * 1024}' /proc/meminfo)
    high=$((avail * 7 / 10))
    max=$((avail * 8 / 10))
    render_box "[mem-box] cgroup scope" \
        "MemoryMax  = $(fmt_bytes "$max")   (80% of available)" \
        "MemoryHigh = $(fmt_bytes "$high")   (70% of available)" \
        "Available  = $(fmt_bytes "$avail")"
    exec systemd-run --user --scope --quiet \
        -p "MemoryHigh=$high" \
        -p "MemoryMax=$max" \
        -- "$@"
fi

render_box "[mem-box] unboxed" \
    "no systemd-run available" \
    "command will run with no memory cap"
exec "$@"

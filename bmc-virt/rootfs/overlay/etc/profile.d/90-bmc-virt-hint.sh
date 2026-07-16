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

# Ensure root has a password so /etc/profile's warning is suppressed.
# chpasswd may be missing from BusyBox; passwd is always available.
grep -qs '^root::' /etc/shadow && {
    printf 'root\nroot\n' | passwd root
} >/dev/null 2>&1

ESC=$(printf '\033')
HEAD="${ESC}[33;1m"
DIM="${ESC}[2m"
RESET="${ESC}[0m"

[ -t 1 ] || return 0
[ -n "${BMC_VIRT_HINT_SHOWN:-}" ] && return 0
export BMC_VIRT_HINT_SHOWN=1

JUST=/usr/bin/just
JUSTFILE=/root/justfile

[ -x "$JUST" ] || return 0
[ -f "$JUSTFILE" ] || return 0

printf '\n'
printf '%s# BMC VM helpers%s\n' "$HEAD" "$RESET"
printf '%sUsage: just <recipe>%s\n' "$DIM" "$RESET"

JUST_COLOR=always "$JUST" --justfile "$JUSTFILE" --list --unsorted

printf '\n'

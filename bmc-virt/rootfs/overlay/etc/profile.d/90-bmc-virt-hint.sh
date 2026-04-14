#!/bin/sh

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

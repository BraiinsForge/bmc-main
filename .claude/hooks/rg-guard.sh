#!/usr/bin/env bash
# rg-guard — PreToolUse(Bash) hook: block ripgrep's `-r`/`--replace` footgun.
#
# `rg -rn 'pat'` parses as `rg --replace=n`: the short `-r` consumes the next
# token as a replacement string and silently rewrites every match. It is almost
# always a typo for `-n` (line numbers). rg's only short `-r` is `--replace`, so
# any short `-r` cluster on an rg call is the footgun; the long form `--replace=`
# is left alone for deliberate use.
#
# Fail-open: any parse problem exits 0 (allow) so the hook can never wedge Bash.

cmd=$(jq -r '.tool_input.command // empty' 2>/dev/null) || exit 0
[ -n "$cmd" ] || exit 0

# `rg` (at a command boundary) carrying a short `-…r…` flag cluster, bounded by
# pipe/;/& so a later `rm -r` in the same line is not mistaken for rg's flag.
if grep -Eq '(^|[|&;[:space:]])rg([[:space:]][^|&;]*)?[[:space:]]-[A-Za-z]*r[A-Za-z]*([[:space:]]|$)' <<<"$cmd"; then
    printf 'rg-guard: `rg -r` is `--replace` and rewrites every match — you almost certainly meant `-n` for line numbers. Use `rg -n`. For deliberate replacement use the long form `--replace=…`.\n' >&2
    exit 2
fi

exit 0

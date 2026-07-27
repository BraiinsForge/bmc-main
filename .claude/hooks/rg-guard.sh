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
    printf 'rg-guard: short -r is --replace and rewrites every match — you almost certainly meant -n for line numbers.\n' >&2
    printf 'Use rg -n. For deliberate replacement use the long form --replace=…\n' >&2
    exit 2
fi

exit 0

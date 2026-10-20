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

# Usage: awk -f summarize.awk <label>-rounds.log
# One row per round: RssAnon and the startup-phase spans in ms, then medians.
function secs(ts) { split(substr(ts, 12, 15), t, ":"); return t[1] * 3600 + t[2] * 60 + t[3] }
function ms(a, b) { return (b - a) * 1000 }
function flush() {
    if (round == "") return
    n++
    v["anon", n] = anon; v["egl", n] = ms(egl0, egl1); v["glscr", n] = ms(gl, scr)
    v["font", n] = ms(scr, init); v["init", n] = ms(start, init); v["widgets", n] = ms(start, w)
    printf "%-6s %3s %8d %8.1f %8.1f %8.1f %8.1f %8.1f\n", label, round, anon, v["egl", n], v["glscr", n], v["font", n], v["init", n], v["widgets", n]
}
function median(key,    i, j, a, tmp) {
    for (i = 1; i <= n; i++) a[i] = v[key, i]
    for (i = 1; i <= n; i++) for (j = i + 1; j <= n; j++) if (a[j] < a[i]) { tmp = a[i]; a[i] = a[j]; a[j] = tmp }
    return n % 2 ? a[(n + 1) / 2] : (a[n / 2] + a[n / 2 + 1]) / 2
}
BEGIN { printf "%-6s %3s %8s %8s %8s %8s %8s %8s\n", "run", "#", "RssAnon", "egl", "gl→scr", "fonts", "→init", "→widgets" }
/^ROUND/ {
    flush()
    label = $2; round = $3
    for (i = 1; i <= NF; i++) if ($i == "RssAnon:") anon = $(i + 1)
    next
}
/starting bmc-wasm-host/ { start = secs($1) }
/Initializing GBM/ { egl0 = secs($1) }
/EGL context created/ { egl1 = secs($1) }
/OpenGL ES:/ { gl = secs($1) }
/FemtoVG screen target/ { scr = secs($1) }
/host renderer initializ/ { init = secs($1) }
/bmc_wasm_host::slot: wasm m/ { w = secs($1) }
END {
    flush()
    printf "%-6s %3s %8d %8.1f %8.1f %8.1f %8.1f %8.1f\n", label, "med", median("anon"), median("egl"), median("glscr"), median("font"), median("init"), median("widgets")
}

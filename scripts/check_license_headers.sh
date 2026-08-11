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

# Verify that every first-party source file carries the GPL license header.
# The exclusion list mirrors "Third-party and generated files" in
# docs/devel/license-headers.md — keep the two in sync.

set -euo pipefail

cd "$(dirname "$0")/.."

# --fix stamps the header from .license.tpl onto every
# flagged file instead of just reporting it.
# Attribution years come from each file's git history.
fix=0
[[ ${1:-} == "--fix" ]] && fix=1

# Patterns matched against repository-relative paths with bash's ==.
excludes=(
    "bmc-shared/ii-net/*"
    "frontend/src/proto/gen/*"
    "frontend/src/lib/react/props.tsx"
    "frontend/src/styles/fonts/*"
    "widgets-wasm-examples/media-control/proto/cast_channel.proto"
    "bmc-virt/kernel-patches/*"
    "bmc-render/keyboard/assets/layouts/*"
    "*/src/manifest_params.rs"
)

source_extensions=()
while IFS= read -r extension; do
    [[ -n $extension ]] && source_extensions+=("$extension")
done <scripts/license_header_extensions.txt

# Tracked files when run from a checkout; plain find inside the nix check
# sandbox, where the source tree is already filtered and has no .git.
list_files() {
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        local globs=()
        for ext in "${source_extensions[@]}"; do
            globs+=("*.$ext")
        done
        git ls-files -z -- "${globs[@]}"
    else
        local names=()
        for ext in "${source_extensions[@]}"; do
            [[ ${#names[@]} -gt 0 ]] && names+=(-o)
            names+=(-name "*.$ext")
        done
        while IFS= read -r -d '' file; do
            printf '%s\0' "${file#./}"
        done < <(find . -type f \( "${names[@]}" \) -print0 | sort -z)
    fi
}

# Line-comment prefix per extension; nonzero for one we can't stamp, so a new
# source type added to the check without a prefix here fails loudly.
comment_prefix() {
    case "${1##*.}" in
    rs | ts | tsx | js | scss | c | proto) printf '//' ;;
    py | sh | nix) printf '#' ;;
    *) return 1 ;;
    esac
}

# Ascending, comma-space-separated year list (`2025, 2026`).
join_years() {
    local IFS=,
    local joined="$*"
    printf '%s' "${joined//,/, }"
}

# Copyright line(s) for a file, split by the 2026 Systems→Forge boundary
# (see docs/devel/license-headers.md). An untracked/new file gets this year.
copyright_lines() {
    local file=$1 prefix=$2 year
    local -a years systems=() forge=()
    mapfile -t years < <(git log --format=%ad --date=format:%Y --follow -- "$file" 2>/dev/null | sort -u)
    [[ ${#years[@]} -eq 0 ]] && years=("$(date +%Y)")
    for year in "${years[@]}"; do
        if ((year <= 2025)); then systems+=("$year"); else forge+=("$year"); fi
    done
    ((${#systems[@]})) && printf '%s Copyright (C) %s  Braiins Systems s.r.o.\n' "$prefix" "$(join_years "${systems[@]}")"
    ((${#forge[@]})) && printf '%s Copyright (C) %s  Braiins Forge s.r.o.\n' "$prefix" "$(join_years "${forge[@]}")"
}

# The full header block: computed copyright line(s), then the boilerplate
# and reservation from .license.tpl verbatim — everything after its templated
# copyright line and the blank below it — so .license.tpl stays the one source.
header_block() {
    local file=$1 prefix=$2 line
    copyright_lines "$file" "$prefix"
    printf '%s\n' "$prefix"
    local -a body
    mapfile -t body < <(awk 'body {print} /^$/ {body = 1}' .license.tpl)
    while ((${#body[@]})) && [[ -z ${body[-1]} ]]; do unset 'body[-1]'; done
    for line in "${body[@]}"; do
        [[ -z $line ]] && printf '%s\n' "$prefix" || printf '%s %s\n' "$prefix" "$line"
    done
}

# Prepend the header, below any shebang, replacing a lone stale copyright line.
stamp_file() {
    local file=$1 prefix i=0
    prefix=$(comment_prefix "$file") || {
        echo "no comment style for: $file" >&2
        return 1
    }
    local -a lines
    mapfile -t lines <"$file"
    local tmp
    tmp=$(mktemp)
    while [[ ${lines[i]:-} == '#!'* ]]; do
        printf '%s\n' "${lines[i]}" >>"$tmp"
        i=$((i + 1))
    done
    if [[ ${lines[i]:-} == "$prefix Copyright (C)"* ]]; then
        i=$((i + 1))
        [[ -z ${lines[i]:-} ]] && i=$((i + 1))
    fi
    header_block "$file" "$prefix" >>"$tmp"
    printf '\n' >>"$tmp"
    local n=${#lines[@]}
    for (( ; i < n; i++)); do printf '%s\n' "${lines[i]}" >>"$tmp"; done
    mv "$tmp" "$file"
}

# Matches both the current boilerplate ("This program is free software...")
# and the upstream BOSI notices kept verbatim in bmc-shared crates
# ("BOSI is free software...").
marker="free software: you can redistribute it and/or modify"
fail=0

while IFS= read -r -d '' file; do
    # Empty files (e.g. bare __init__.py) carry no header.
    [[ -s $file ]] || continue

    for pattern in "${excludes[@]}"; do
        # shellcheck disable=SC2053 # pattern must glob-match, not literal-match
        if [[ $file == $pattern ]]; then
            continue 2
        fi
    done

    header=$(head -n 30 "$file")
    if [[ $header != *"$marker"* ]]; then
        if ((fix)); then
            stamp_file "$file" && echo "stamped: $file"
        else
            echo "missing license header: $file"
            fail=1
        fi
    fi
done < <(list_files)

exit "$fail"

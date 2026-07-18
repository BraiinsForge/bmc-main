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

# Patterns matched against repository-relative paths with bash's ==.
excludes=(
    "bmc-shared/ii-net/*"
    "frontend/src/proto/gen/*"
    "frontend/src/lib/react/props.tsx"
    "frontend/src/styles/fonts/*"
    "bmc-wasm-runtime/examples/media-control/proto/cast_channel.proto"
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
        echo "missing license header: $file"
        fail=1
    fi
done < <(list_files)

exit "$fail"

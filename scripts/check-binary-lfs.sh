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

# Verify every binary file added/modified on the current branch is tracked
# via Git LFS, both at the .gitattributes layer (the file's extension routes
# through `filter=lfs`) and at the storage layer (the stored blob is an LFS
# pointer, not raw bytes).
#
# Compares HEAD against the merge-base with the target branch (origin/master
# by default; the GitLab MR base SHA when running in an MR pipeline).
#
# Exits non-zero with the list of offending paths if any binary blob landed
# without LFS coverage. Designed for CI; safe to run locally too.

set -euo pipefail

# Pick the base commit. MR pipelines get a precomputed base from GitLab;
# everything else falls back to merge-base with master.
if [[ -n ${CI_MERGE_REQUEST_DIFF_BASE_SHA:-} ]]; then
    base="$CI_MERGE_REQUEST_DIFF_BASE_SHA"
else
    base="$(git merge-base HEAD origin/master 2>/dev/null || git merge-base HEAD master)"
fi

head_short="$(git rev-parse --short HEAD)"
base_short="$(git rev-parse --short "$base")"
echo "binary-lfs check: comparing HEAD ($head_short) against base ($base_short)"

# `git diff --numstat` reports "-<TAB>-<TAB><path>" for binary files; use that
# as the binary detector for the entire branch diff in a single call.
mapfile -t binaries < <(
    git diff --numstat --diff-filter=AM "$base"...HEAD \
        | awk -F'\t' '$1 == "-" && $2 == "-" { print $3 }'
)

if ((${#binaries[@]} == 0)); then
    echo "binary-lfs check: OK (no binary files added or modified)"
    exit 0
fi

# LFS pointer files start with this fixed version line. The whole file is
# typically <140 bytes; 256 is a safe sampling upper bound.
lfs_pointer_prefix='version https://git-lfs.github.com/spec/v1'
sample_bytes=256

violations=()
for path in "${binaries[@]}"; do
    # Defensive: --diff-filter=AM should never yield a deleted path, but skip
    # if the file is gone from the worktree for any reason.
    [[ -e $path ]] || continue

    # JavaScript is text even when .gitattributes marks it `binary` to suppress
    # diffs (e.g. Yarn's bundled runtime under .yarn/) — it is never LFS material.
    case $path in
    *.cjs | *.mjs | *.js) continue ;;
    esac

    filter="$(git check-attr filter -- "$path" | awk -F': ' '{print $NF}')"
    if [[ $filter != "lfs" ]]; then
        violations+=("$path  (.gitattributes does not route through LFS)")
        continue
    fi

    # Stored blob must be an LFS pointer, not raw bytes.
    head_bytes="$(git show "HEAD:$path" 2>/dev/null | head -c "$sample_bytes" || true)"
    if [[ $head_bytes != "$lfs_pointer_prefix"* ]]; then
        violations+=("$path  (matches LFS filter but is stored as raw bytes — run 'git lfs migrate import')")
    fi
done

if ((${#violations[@]} > 0)); then
    {
        echo
        echo "binary files lack LFS coverage:"
        for v in "${violations[@]}"; do
            echo "  - $v"
        done
        echo
        echo "Fix by adding the extension to .gitattributes, e.g.:"
        echo "    *.<ext> filter=lfs diff=lfs merge=lfs -text"
        echo "and re-importing the offending blob into LFS:"
        echo "    git lfs migrate import --include='*.<ext>' --include-ref=refs/heads/<branch>"
    } >&2
    exit 1
fi

echo "binary-lfs check: OK (${#binaries[@]} binary files inspected)"

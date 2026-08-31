#!/usr/bin/env python3
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

"""Check the gallery's lint parity and scene-source completeness.

`bmc-gallery` is its own workspace root so its lockfile and egui version stay
clear of ours, which also means Cargo cannot inherit `[workspace.lints]` across
the boundary — the set is restated in its manifest instead. Nothing enforces
that copy, so a lint added at the root would quietly stop applying to the
scenes. This compares the lint tables and fails when they disagree.
The filtered Nix source must also contain every expected gallery scene.

Deviations are allowed, but they are declared here rather than discovered.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
# Nix filters this check to declared gallery dependencies.
# An independent total detects scenes that disappear from that source closure.
EXPECTED_SCENE_COUNT = 39

# Lints the gallery departs from the root on, each with its reason recorded
# beside the entry in `bmc-gallery/Cargo.toml`. Anything else that differs is drift.
DEVIATIONS = {('rust', 'elided_lifetimes_in_paths')}


def workspace_lints(manifest: Path) -> dict[str, dict[str, object]]:
    """The `[workspace.lints]` tables of a manifest, keyed by lint group."""
    with manifest.open('rb') as handle:
        return tomllib.load(handle).get('workspace', {}).get('lints', {})


def main() -> int:
    root = workspace_lints(REPO / 'Cargo.toml')
    gallery = workspace_lints(REPO / 'bmc-gallery' / 'Cargo.toml')
    scene_count = sum(1 for _ in REPO.glob('**/*.scene.rs'))

    scene_count_matches = scene_count == EXPECTED_SCENE_COUNT
    if not scene_count_matches:
        print(
            f'gallery source contains {scene_count} scenes; '
            f'expected {EXPECTED_SCENE_COUNT}',
            file=sys.stderr,
        )
        print(
            'remove stray or WIP scene files, or add a filtered-out scene crate '
            'to bmc-gallery dependencies; update EXPECTED_SCENE_COUNT only for '
            'an intentional scene-set change',
            file=sys.stderr,
        )

    drift = []
    for group in sorted(set(root) | set(gallery)):
        at_root, at_gallery = root.get(group, {}), gallery.get(group, {})
        for lint in sorted(set(at_root) | set(at_gallery)):
            if (group, lint) in DEVIATIONS:
                continue
            if at_root.get(lint) != at_gallery.get(lint):
                drift.append(
                    f'  {group}.{lint}: root={at_root.get(lint)!r}, '
                    f'gallery={at_gallery.get(lint)!r}'
                )

    if drift:
        print(
            "bmc-gallery's lint set has drifted from the workspace root:",
            file=sys.stderr,
        )
        print('\n'.join(drift), file=sys.stderr)
        print(
            '\nMirror the change into bmc-gallery/Cargo.toml, or — if the gallery '
            'should differ — add it to DEVIATIONS here and say why in the manifest.',
            file=sys.stderr,
        )
    if not scene_count_matches or drift:
        return 1

    print(
        f'lint sets match across {len(root)} groups; '
        f'gallery source contains all {scene_count} scenes'
    )
    return 0


if __name__ == '__main__':
    sys.exit(main())

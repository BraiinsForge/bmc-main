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

"""Host-side proof of the B2 ext4 corruption recipe (BDK-601).

The pinned recipe must deterministically drive partition.rs's fsck
ladder into its repair branch: `e2fsck -p` (no -f) exits 4 (corrupt →
escalate), `e2fsck -y` repairs with exit 1, blkid still sees ext4, and
the UUID survives (repaired in place, never reformatted). Validated by
hand on e2fsprogs 1.47.3; a divergence here is a finding to
investigate, never an accepted alternative outcome.
"""

import re
import shutil
import subprocess
from pathlib import Path

import pytest

from bmc_tui import catalog

_TOOLS = ("mkfs.ext4", "debugfs", "e2fsck", "blkid")

pytestmark = pytest.mark.skipif(
    any(shutil.which(t) is None for t in _TOOLS),
    reason="e2fsprogs/util-linux missing — run inside `nix develop` (CI shell has them)",
)


def _run(argv: list[str]) -> "subprocess.CompletedProcess[str]":
    return subprocess.run(argv, capture_output=True, text=True, check=False)


def _uuid(image: Path) -> str:
    out = _run(["blkid", str(image)]).stdout
    match = re.search(r'UUID="([^"]+)"', out)
    assert match is not None, f"blkid reports no UUID: {out!r}"
    return match.group(1)


@pytest.fixture
def ext4_image(tmp_path: Path) -> Path:
    image = tmp_path / "disk.img"
    with image.open("wb") as f:
        f.truncate(8 * 1024 * 1024)
    mkfs = _run(["mkfs.ext4", "-F", "-q", str(image)])
    assert mkfs.returncode == 0, mkfs.stderr
    return image


def test_corrupt_fs_metadata_recipe_drives_the_repair_branch(ext4_image: Path) -> None:
    uuid_before = _uuid(ext4_image)

    for command in catalog._B2_DEBUGFS_COMMANDS:
        applied = _run(["debugfs", "-w", "-R", command, str(ext4_image)])
        assert applied.returncode == 0, applied.stderr

    blkid = _run(["blkid", str(ext4_image)]).stdout
    assert 'TYPE="ext4"' in blkid, f"blkid no longer sees ext4: {blkid!r}"

    preen = _run(["e2fsck", "-p", str(ext4_image)])
    assert preen.returncode == 4, (
        f"e2fsck -p exited {preen.returncode}, expected 4 (unfixed errors → the "
        f"ladder must escalate): {preen.stdout} {preen.stderr}"
    )

    repair = _run(["e2fsck", "-y", str(ext4_image)])
    assert repair.returncode == 1, (
        f"e2fsck -y exited {repair.returncode}, expected 1 (errors corrected): "
        f"{repair.stdout} {repair.stderr}"
    )

    assert _uuid(ext4_image) == uuid_before, "the repair must not change the filesystem identity"

    clean = _run(["e2fsck", "-p", str(ext4_image)])
    assert clean.returncode == 0, "the repaired filesystem must pass preen cleanly"

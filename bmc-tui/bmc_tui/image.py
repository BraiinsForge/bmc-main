"""A local firmware sysupgrade tarball and the metadata read from it.

The board/version live in the tar's ``COMMAND`` file (``UPGRADE_FW_VERSION``),
not in ``version.json`` (which is the bosminer-toml schema marker). The
authoritative compatibility check runs on the device during sysupgrade.
"""

import tarfile
from dataclasses import dataclass
from pathlib import Path

from bmc_tui import console

_REMOTE_DIR = "/mnt/data"


@dataclass
class Image:
    """A firmware sysupgrade tarball on the local filesystem."""

    path: Path

    @property
    def size(self) -> int:
        return self.path.stat().st_size

    @property
    def remote_path(self) -> str:
        return f"{_REMOTE_DIR}/{self.path.name}"

    def print(self) -> None:
        """Print a short image summary for a run preamble."""
        console.kv("image", self.path.name)
        console.kv("size", console.human_size(self.size))

    def members(self) -> list[str]:
        try:
            with tarfile.open(self.path) as tar:
                return tar.getnames()
        except tarfile.TarError:
            return []  # not a readable tar — callers treat it as "not a sysupgrade image"

    @property
    def sysupgrade_dir(self) -> str | None:
        """The single top-level ``sysupgrade-*`` directory, or None if the tar
        does not have exactly one."""
        tops = {name.split("/", 1)[0] for name in self.members()}
        dirs = sorted(t for t in tops if t.startswith("sysupgrade-"))
        return dirs[0] if len(dirs) == 1 else None

    @property
    def is_sysupgrade(self) -> bool:
        """True if the tar looks like a Deck sysupgrade image: one
        ``sysupgrade-*`` dir containing COMMAND and rootfs.img."""
        directory = self.sysupgrade_dir
        if directory is None:
            return False
        names = set(self.members())
        return all(f"{directory}/{f}" in names for f in ("COMMAND", "rootfs.img"))

    @property
    def version(self) -> str:
        """Firmware version — ``UPGRADE_FW_VERSION`` from the tar's COMMAND."""
        for line in self._command().splitlines():
            if line.startswith("UPGRADE_FW_VERSION="):
                return line.split("=", 1)[1].strip().strip('"')
        msg = f"UPGRADE_FW_VERSION not found in {self.path.name}"
        raise ValueError(msg)

    def _command(self) -> str:
        with tarfile.open(self.path) as tar:
            member = tar.extractfile(f"{self.sysupgrade_dir}/COMMAND")
            return member.read().decode() if member is not None else ""

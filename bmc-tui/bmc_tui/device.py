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

"""SSH transport to a real device, with a dry-run-aware exec seam.

`read` is for read-only probes — it always executes, so probes reflect real
device state even under `--dry-run`. `run`/`push` are mutations — under
`--dry-run` they log and skip. Routine probes are exposed as getters
(`reachable`, `board`, `version`) so call-sites never type raw ssh for them.

All subprocess work goes through an injected `Exec` backend (`run` captures,
`stream` feeds stdin) so tests need no real ssh.
"""

import json
import shlex
import subprocess
from collections.abc import Iterable, Iterator
from pathlib import Path
from typing import Any, NewType, Protocol

from bmc_tui import console
from bmc_tui.stage import dry_run

# Key-based, accept-new host key, no password — unlike the VM's sshpass path.
_SSH_OPTS = [
    "-o",
    "StrictHostKeyChecking=accept-new",
    "-o",
    "ConnectTimeout=8",
    # ConnectTimeout only guards the connect phase; a session whose peer
    # vanishes mid-command otherwise hangs forever. Keepalives turn that
    # into ssh's exit 255 within ~15s, which expect_disconnect handles.
    "-o",
    "ServerAliveInterval=5",
    "-o",
    "ServerAliveCountMax=3",
    # quiet ssh's post-quantum-KEX warning
    "-o",
    "LogLevel=ERROR",
]

_CHUNK = 1 << 16  # 64 KiB upload chunk

# An absolute path on the device, as opposed to a local `Path`.
# `push` already takes one of each; this names the distinction.
RemotePath = NewType("RemotePath", str)

# Exit codes >= 128 mean the session died rather than the command failing:
# 255 is ssh's own code for a dropped connection, and anything else in the
# range encodes remote signal death (128+N, or a negative -N wrapped to an
# unsigned byte) — what a reboot's shutdown does to the live session.
_SESSION_DEATH_FLOOR = 128


class Exec(Protocol):
    """Subprocess backend Device runs through — injected so tests need no ssh."""

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        """Run a command to completion and capture its output."""
        ...

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        """Run a command, feeding `chunks` to its stdin."""
        ...


class _RealExec:
    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        return subprocess.run(argv, capture_output=True, text=True, check=True)

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        # Capture stderr so ssh's connection warnings don't bleed into the live
        # progress bar; surface it only if the upload actually fails.
        with subprocess.Popen(argv, stdin=subprocess.PIPE, stderr=subprocess.PIPE) as proc:
            stdin = proc.stdin
            if stdin is None:
                msg = "BUG: Popen(stdin=PIPE) produced no stdin"
                raise RuntimeError(msg)
            for chunk in chunks:
                stdin.write(chunk)
            stdin.close()
            stderr = proc.stderr.read() if proc.stderr is not None else b""
        if proc.returncode:
            raise subprocess.CalledProcessError(proc.returncode, argv, stderr=stderr)


class Device:
    """A target device addressed over key-based SSH."""

    def __init__(self, host: str, *, user: str = "root", backend: Exec | None = None) -> None:
        self._host = host
        self._user = user
        self._exec: Exec = backend or _RealExec()
        self._info: dict[str, Any] | None = None

    @property
    def host(self) -> str:
        return self._host

    @property
    def login(self) -> str:
        """`user@host` SSH login, always explicit — for remediation hints and
        commands the operator copy-pastes (unlike `print`, which elides root)."""
        return f"{self._user}@{self._host}"

    @property
    def copy_dest(self) -> str:
        """`nix copy --to` URI driving the device's own nix-store binary, so the
        device needs only the store, not a full nix."""
        return f"ssh://{self._user}@{self._host}?remote-program=/run/current-profile/bin/nix-store"

    def print(self) -> None:
        """Print a one-line device summary for a run preamble."""
        target = self._host if self._user == "root" else f"{self._user}@{self._host}"
        console.kv("device", target)

    def _ssh_argv(self, command: str) -> list[str]:
        return ["ssh", *_SSH_OPTS, f"{self._user}@{self._host}", command]

    def read(self, command: str) -> str:
        """Run a read-only command and return its stripped stdout. Always runs,
        even under --dry-run, so probes reflect real device state."""
        return self._exec.run(self._ssh_argv(command)).stdout.strip()

    def run(self, command: str, *, expect_disconnect: bool = False) -> str | None:
        """Run a mutating command and return its stripped stdout. Under
        --dry-run, log and skip, returning None.

        ``expect_disconnect`` treats a killed session (exit >= 128: ssh's
        own 255 for a dropped connection, or a remote session killed by the
        shutdown's signal) as success — for commands that intentionally
        take the device down (e.g. ``sysupgrade``) — and returns None
        since no output came back. A remote command failing outright
        (small exit code, session alive) still raises.
        """
        if dry_run.get():
            console.kv("would run", command)
            return None
        try:
            return self._exec.run(self._ssh_argv(command)).stdout.strip()
        except subprocess.CalledProcessError as e:
            if not (expect_disconnect and e.returncode >= _SESSION_DEATH_FLOOR):
                raise
            return None

    def push(self, local: Path, remote: RemotePath) -> None:
        """Upload a file, streamed over ssh with a live progress bar. Under
        --dry-run, log and skip."""
        if dry_run.get():
            console.kv("would upload", f"{local.name} -> {remote}")
            return
        argv = ["ssh", *_SSH_OPTS, f"{self._user}@{self._host}", f"cat > {shlex.quote(remote)}"]
        self._exec.stream(argv, _chunks(local))

    def extract_tar(self, local: Path) -> None:
        """Stream a local tarball into `tar xzf - -C /` on the device, with a
        live progress bar — no intermediate copy. Under --dry-run, log and skip."""
        if dry_run.get():
            console.kv("would extract", f"{local.name} -> /")
            return
        argv = ["ssh", *_SSH_OPTS, f"{self._user}@{self._host}", "tar xzf - -C /"]
        self._exec.stream(argv, _chunks(local))

    @property
    def reachable(self) -> bool:
        """True if SSH answers. Read-only, safe under --dry-run."""
        try:
            self.read("true")
        except (subprocess.SubprocessError, OSError):
            return False
        return True

    def _board_info(self) -> dict[str, Any]:
        """``ubus call system board``, parsed and cached (immutable per device)."""
        if self._info is None:
            self._info = json.loads(self.read("ubus call system board"))
        return self._info

    @property
    def board(self) -> str:
        """Board id, e.g. ``braiins,stm32mp157c-ii3-bmc1``."""
        return self._board_info()["board_name"]

    @property
    def target(self) -> str:
        """Release target, e.g. ``stm32mp15/ii3`` — used for image board-family
        validation."""
        return self._board_info()["release"]["target"]

    @property
    def version(self) -> str:
        """Installed firmware version. Not cached — sysupgrade changes it across
        the reboot, so each access re-reads the live value."""
        return self.read("cat /etc/bos_version")


def _chunks(local: Path) -> Iterator[bytes]:
    """Yield the file in chunks, advancing a byte-count progress bar as each is sent."""
    with local.open("rb") as f, console.progress(local.name, local.stat().st_size) as advance:
        while chunk := f.read(_CHUNK):
            yield chunk
            advance(len(chunk))

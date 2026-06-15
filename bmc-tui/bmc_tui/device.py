"""SSH/SCP transport to a real device, with a dry-run-aware exec seam.

`read` is for read-only probes — it always executes, so probes reflect real
device state even under `--dry-run`. `run`/`push` are mutations — under
`--dry-run` they log and skip. Routine probes are exposed as getters
(`reachable`, `board`, `version`) so call-sites never type raw ssh for them.
"""

import json
import subprocess
from collections.abc import Callable
from pathlib import Path
from typing import Any

from bmc_tui import console
from bmc_tui.stage import dry_run

# Key-based, accept-new host key, no password — unlike the VM's sshpass path.
_SSH_OPTS = ["-o", "StrictHostKeyChecking=accept-new", "-o", "ConnectTimeout=8"]

# The single point where a subprocess actually runs; tests inject a fake.
Runner = Callable[[list[str]], "subprocess.CompletedProcess[str]"]


def _subprocess_runner(argv: list[str]) -> "subprocess.CompletedProcess[str]":
    return subprocess.run(argv, capture_output=True, text=True, check=True)


class Device:
    """A target device addressed over key-based SSH."""

    def __init__(self, host: str, *, user: str = "root", runner: Runner | None = None) -> None:
        self._host = host
        self._user = user
        self._exec: Runner = runner or _subprocess_runner
        self._info: dict[str, Any] | None = None

    @property
    def host(self) -> str:
        return self._host

    def print(self) -> None:
        """Print a one-line device summary for a run preamble."""
        target = self._host if self._user == "root" else f"{self._user}@{self._host}"
        console.kv("device", target)

    def _ssh_argv(self, command: str) -> list[str]:
        return ["ssh", *_SSH_OPTS, f"{self._user}@{self._host}", command]

    def read(self, command: str) -> str:
        """Run a read-only command and return its stripped stdout. Always runs,
        even under --dry-run, so probes reflect real device state."""
        return self._exec(self._ssh_argv(command)).stdout.strip()

    def run(self, command: str, *, expect_disconnect: bool = False) -> None:
        """Run a mutating command. Under --dry-run, log and skip.

        ``expect_disconnect`` treats a dropped connection as success — for
        commands that intentionally kill the session (e.g. ``sysupgrade``).
        """
        if dry_run.get():
            console.kv("would run", command)
            return
        try:
            self._exec(self._ssh_argv(command))
        except subprocess.SubprocessError:
            if not expect_disconnect:
                raise

    def push(self, local: Path, remote: str) -> None:
        """Upload a file via scp. Under --dry-run, log and skip."""
        if dry_run.get():
            console.kv("would upload", f"{local} -> {remote}")
            return
        self._exec(["scp", *_SSH_OPTS, str(local), f"{self._user}@{self._host}:{remote}"])

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

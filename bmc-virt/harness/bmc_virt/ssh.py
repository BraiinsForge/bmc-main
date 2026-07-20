"""Self-contained SSH/SCP via subprocess.

Replicates the credentials and options from bmc-virt/scripts/_.sh but without
any dependency on those shell scripts.
"""

import shlex
import subprocess
from pathlib import Path

# ── Defaults ───────────────────────────────────────────────────────────────────

DEFAULT_HOST = "localhost"
DEFAULT_PORT = 2222
DEFAULT_USER = "root"
DEFAULT_PASSWORD = "root"

# SSH options matching scripts/_.sh — no config, no host key checking
_SSH_BASE_OPTS = [
    "-F",
    "/dev/null",
    "-o",
    "StrictHostKeyChecking=no",
    "-o",
    "UserKnownHostsFile=/dev/null",
    "-o",
    "WarnWeakCrypto=no",
]


class Ssh:
    """SSH/SCP operations against a running bmc-virt VM."""

    def __init__(
        self,
        *,
        host: str = DEFAULT_HOST,
        port: int = DEFAULT_PORT,
        user: str = DEFAULT_USER,
        password: str = DEFAULT_PASSWORD,
    ) -> None:
        self._host = host
        self._port = port
        self._user = user
        self._password = password

    def _ssh_cmd(self, command: str, *, tty: bool = False) -> list[str]:
        """Build the full ssh command line."""
        cmd = ["sshpass", "-p", self._password, "ssh"]
        cmd.extend(_SSH_BASE_OPTS)
        cmd.extend(["-p", str(self._port)])
        if tty:
            cmd.append("-t")
        cmd.append(f"{self._user}@{self._host}")
        cmd.append(command)
        return cmd

    def _scp_cmd(self, src: str, dst: str, *, recursive: bool = False) -> list[str]:
        """Build the full scp command line."""
        cmd = ["sshpass", "-p", self._password, "scp"]
        cmd.extend(_SSH_BASE_OPTS)
        if recursive:
            cmd.append("-r")
        cmd.extend(["-P", str(self._port), "-O"])
        cmd.extend([src, dst])
        return cmd

    def run(
        self,
        command: str,
        *,
        timeout: float = 30,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        """Execute a command on the VM via SSH."""
        return subprocess.run(
            self._ssh_cmd(command),
            capture_output=True,
            text=True,
            timeout=timeout,
            check=check,
        )

    def pull(self, *, remote: str, local: str | Path) -> None:
        """Copy a file or directory from the VM to the host."""
        local = Path(local)
        local.parent.mkdir(parents=True, exist_ok=True)
        src = f"{self._user}@{self._host}:{remote}"
        subprocess.run(
            self._scp_cmd(src, str(local), recursive=True),
            check=True,
            capture_output=True,
            text=True,
        )

    def push(self, *, local: str | Path, remote: str) -> None:
        """Copy a file or directory from the host to the VM."""
        # scp cannot create the remote parent directory; ensure it exists first
        self.run(f'mkdir -p "$(dirname {shlex.quote(remote)})"')
        dst = f"{self._user}@{self._host}:{remote}"
        subprocess.run(
            self._scp_cmd(str(local), dst, recursive=True),
            check=True,
            capture_output=True,
            text=True,
        )

    def probe(self, *, timeout: float = 2) -> bool:
        """Check if SSH is responsive (non-blocking)."""
        try:
            result = self.run("true", timeout=timeout, check=False)
        except subprocess.TimeoutExpired:
            return False
        return result.returncode == 0

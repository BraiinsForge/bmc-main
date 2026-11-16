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
The run's shared connections (see `ssh`) take their own runner for their control commands.
Outside a run, `forward` spawns the ssh holding the tunnel itself,
as the tunnel must keep listening while the caller dials it.
"""

import shlex
import socket
import subprocess
import time
import uuid
from collections.abc import Callable, Generator, Iterable, Iterator
from contextlib import contextmanager, suppress
from dataclasses import dataclass
from pathlib import Path
from typing import IO, Literal, NewType, Protocol, TypedDict, cast

import msgspec
from rich.markup import escape

from bmc_tui import console, ssh
from bmc_tui.process import killed_if_cut_short, stream_lines
from bmc_tui.stage import Abort, dry_run, require

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
_WINDOW_MARK = "bmc-tui-log-window"

# Exit codes >= 128 mean the session died rather than the command failing:
# 255 is ssh's own code for a dropped connection, and anything else in the
# range encodes remote signal death (128+N, or a negative -N wrapped to an
# unsigned byte) — what a reboot's shutdown does to the live session.
_SESSION_DEATH_FLOOR = 128


_FORWARD_TIMEOUT_S = 15.0


def _await_listening(proc: "subprocess.Popen[str]", port: int, what: str) -> None:
    deadline = time.monotonic() + _FORWARD_TIMEOUT_S
    while True:
        # Once ssh has ended, what listens on the port is not its forward
        if proc.poll() is not None:
            stderr_pipe: IO[str] | None = proc.stderr
            stderr = stderr_pipe.read().strip() if stderr_pipe is not None else ""
            raise Abort(f"{what} failed (ssh exit {proc.returncode}): {stderr}")
        if _listening(port):
            return
        if time.monotonic() >= deadline:
            raise Abort(f"{what}: no listener within {_FORWARD_TIMEOUT_S:.0f}s")
        time.sleep(0.1)


def _free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        _host, port = cast("tuple[str, int]", probe.getsockname())
    return port


def _listening(port: int) -> bool:
    try:
        socket.create_connection(("127.0.0.1", port), timeout=1).close()
    except OSError:
        return False
    return True


def _is_session_death(returncode: int) -> bool:
    """Whether an ssh exit encodes a killed session rather than a failed
    command: ssh's own 255 for a dropped connection, remote signal death
    (128+N), or the negative -N `subprocess.run` reports for local signal
    death. Shared so `run(expect_disconnect=True)` and `run_captured` agree."""
    return returncode < 0 or returncode >= _SESSION_DEATH_FLOOR


@dataclass(frozen=True)
class RunOutcome:
    """What a captured remote command did — including the streams, which
    the plain `run(expect_disconnect=True)` path discards on session
    death."""

    status: Literal["clean", "failed", "session-death"]
    returncode: int
    output: str  # stdout + stderr


def _captured_text(captured: str | bytes | None) -> str:
    if captured is None:
        return ""
    if isinstance(captured, bytes):
        return captured.decode(errors="replace")
    return captured


class Exec(Protocol):
    """Subprocess backend Device runs through — injected so tests need no ssh."""

    def run(self, argv: list[str], /) -> "subprocess.CompletedProcess[str]":
        """Run a command to completion and capture its output."""
        ...

    def stream(self, argv: list[str], chunks: Iterable[bytes], /) -> None:
        """Run a command, feeding `chunks` to its stdin."""
        ...

    def stream_output(self, argv: list[str], on_line: Callable[[str], None], /) -> int:
        """Run a command, calling `on_line` for each output line as it arrives;
        return the exit code without raising on a nonzero one."""
        ...


class _RealExec:
    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        return subprocess.run(argv, capture_output=True, text=True, check=True)

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        # Capture stderr so ssh's connection warnings don't bleed into the live
        # progress bar; surface it only if the upload actually fails.
        with (
            subprocess.Popen(argv, stdin=subprocess.PIPE, stderr=subprocess.PIPE) as proc,
            killed_if_cut_short(proc),
        ):
            stdin = proc.stdin
            if stdin is None:
                msg = "BUG: Popen(stdin=PIPE) produced no stdin"
                raise RuntimeError(msg)
            for chunk in chunks:
                _ = stdin.write(chunk)
            stdin.close()
            stderr = proc.stderr.read() if proc.stderr is not None else b""
        if proc.returncode:
            raise subprocess.CalledProcessError(proc.returncode, argv, stderr=stderr)

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        return stream_lines(argv, on_line)


class BoardRelease(TypedDict):
    target: str


class BoardInfo(TypedDict):
    board_name: str
    release: BoardRelease


class Device:
    """A target device addressed over SSH, with a key or a password (see `ssh`).

    It shares the run's connections unless given its own `connections`, or none when given `None`.
    """

    def __init__(
        self,
        host: str,
        *,
        user: str = "root",
        backend: Exec | None = None,
        connections: ssh.SharedConnections | ssh.FromTheRun | None = ssh.FROM_THE_RUN,
    ) -> None:
        self._host: str = host
        self._user: str = user
        self._exec: Exec = backend or _RealExec()
        # Taken now, as a worker thread the device is handed to does not see the run's
        self._connections: ssh.SharedConnections | None = (
            ssh.connections.get() if isinstance(connections, ssh.FromTheRun) else connections
        )
        self._info: BoardInfo | None = None

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

    def join_options(self) -> list[str]:
        """
        Options for a program running ssh itself to join the device's shared connection.
        Empty outside a run.
        """
        if self._connections is None:
            return []
        return self._connections.join_options(self.login)

    def _ssh_opts(self) -> list[str]:
        if self._connections is None:
            return [*_SSH_OPTS]
        return [*self._connections.options(self.login), *_SSH_OPTS]

    def _ssh_argv(self, command: str) -> list[str]:
        return ["ssh", *self._ssh_opts(), self.login, command]

    @contextmanager
    def forward(self, remote_port: int) -> Generator[str]:
        """Yield a local `host:port` that SSH forwards to `remote_port` on the device.

        Reaches the device wherever SSH does, ssh-config aliases and jump hosts included.
        """
        what = f"forwarding {self.login}:{remote_port}"
        if self._connections is None:
            with self._own_tunnel(remote_port, what) as address:
                yield address
            return
        # Opens the connection, so a password asked for here has all the time it needs
        try:
            _ = self._exec.run(self._ssh_argv("true"))
        except subprocess.CalledProcessError as e:
            said = _captured_text(cast("str | bytes | None", e.stderr)).strip()
            raise Abort(f"{what} failed (ssh exit {e.returncode}): {said}") from None
        # Picked only now, so the time a password takes cannot let another process take it
        local_port = _free_port()
        spec = f"127.0.0.1:{local_port}:127.0.0.1:{remote_port}"
        try:
            self._connections.add_forward(self.login, spec)
        except ssh.ControlError as e:
            raise Abort(f"{what}: {e}") from None
        try:
            yield f"127.0.0.1:{local_port}"
        except BaseException:
            try:
                self._cancel_forward(self._connections, spec, local_port)
            except Exception as e:
                # A terminal that hung up must not replace what cut the block short
                with suppress(OSError):
                    console.warn(escape(f"{what}: cancelling the forward failed too: {e}"))
            raise
        else:
            try:
                self._cancel_forward(self._connections, spec, local_port)
            except ssh.ControlError as e:
                raise Abort(f"{what}: {e}") from None

    def _end_connections(self) -> None:
        # Every login of the run, as a run addresses one device, by name and by its pinned address.
        # A connection left up would wait out its keepalives and hold up the next ssh for as long.
        if self._connections is None:
            return
        try:
            self._connections.end_all()
        except ssh.ControlError as e:
            # A terminal that hung up must not fail a flash that went through
            with suppress(OSError):
                console.warn(
                    escape(f"{e}; still open, so the next ssh to it may wait for it to time out")
                )

    def _cancel_forward(self, connections: ssh.SharedConnections, spec: str, port: int) -> None:
        connections.cancel_forward(self.login, spec)
        # `ssh -O cancel` exits with 0 even when the connection refused it
        if _listening(port):
            msg = f"`ssh -O cancel` left 127.0.0.1:{port} listening"
            raise ssh.ControlError(msg)

    @contextmanager
    def _own_tunnel(self, remote_port: int, what: str) -> Generator[str]:
        local_port = _free_port()
        spec = f"127.0.0.1:{local_port}:127.0.0.1:{remote_port}"
        # No control socket, so a master from the user's ssh config cannot take the forward over
        argv = ["ssh", *self._ssh_opts(), "-o", "ControlPath=none"]
        argv += ["-o", "ExitOnForwardFailure=yes", "-N", "-L", spec]
        with subprocess.Popen(
            [*argv, self.login],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        ) as proc:
            try:
                _await_listening(proc, local_port, what)
                yield f"127.0.0.1:{local_port}"
            finally:
                proc.terminate()

    def read(self, command: str) -> str:
        """Run a read-only command and return its stripped stdout. Always runs,
        even under --dry-run, so probes reflect real device state."""
        return self._exec.run(self._ssh_argv(command)).stdout.strip()

    def run(self, command: str, *, expect_disconnect: bool = False) -> str | None:
        """Run a mutating command and return its stripped stdout. Under
        --dry-run, log and skip, returning None.

        ``expect_disconnect`` treats a killed session (see
        ``_is_session_death``: ssh's own 255 for a dropped connection, a
        remote session killed by the shutdown's signal, or local signal
        death) as success — for commands that intentionally
        take the device down (e.g. ``sysupgrade``) — and returns None
        since no output came back. A remote command failing outright
        (small exit code, session alive) still raises.
        With it, a command that does not fail takes the device down, and the run's connections end.
        """
        if dry_run.get():
            console.kv("would run", command)
            return None
        try:
            stdout = self._exec.run(self._ssh_argv(command)).stdout.strip()
        except subprocess.CalledProcessError as e:
            if not (expect_disconnect and _is_session_death(e.returncode)):
                raise
            stdout = None
        if expect_disconnect:
            self._end_connections()
        return stdout

    def run_streamed(
        self,
        command: str,
        *,
        on_line: Callable[[str], None],
        expect_disconnect: bool = False,
    ) -> None:
        """Stream a mutating command's merged stdout+stderr to `on_line` line by
        line — nothing is captured. Under --dry-run, log and skip. Exit handling
        mirrors `run` (disconnect is success when `expect_disconnect`); a raised
        `CalledProcessError` carries no output, since it was streamed."""
        if dry_run.get():
            console.kv("would run", command)
            return
        argv = self._ssh_argv(command)
        code = self._exec.stream_output(argv, on_line)
        went_down = expect_disconnect and (code == 0 or _is_session_death(code))
        if went_down:
            self._end_connections()
        if code == 0 or went_down:
            return
        raise subprocess.CalledProcessError(code, argv)

    def run_captured(self, command: str) -> RunOutcome | None:
        """Execute a flash `command`, preserving outcome + output instead of raising.
        Session death (ssh exit >= 128, or signal death) is a distinct
        outcome, not an error — the flash stages decide what it means.
        Unless it failed, the device goes down, and the run's connections end.
        Returns None under --dry-run."""
        if dry_run.get():
            console.kv("would run", command)
            return None
        try:
            proc = self._exec.run(self._ssh_argv(command))
        except subprocess.CalledProcessError as e:
            died = _is_session_death(e.returncode)
            if died:
                self._end_connections()
            return RunOutcome(
                status="session-death" if died else "failed",
                returncode=e.returncode,
                output=_captured_text(cast("str | bytes | None", e.stdout))
                + _captured_text(cast("str | bytes | None", e.stderr)),
            )
        self._end_connections()
        return RunOutcome(
            status="clean", returncode=proc.returncode, output=proc.stdout + proc.stderr
        )

    def push(self, local: Path, remote: RemotePath) -> None:
        """Upload a file, streamed over ssh with a live progress bar. Under
        --dry-run, log and skip."""
        if dry_run.get():
            console.kv("would upload", f"{local.name} -> {remote}")
            return
        argv = ["ssh", *self._ssh_opts(), self.login, f"cat > {shlex.quote(remote)}"]
        self._exec.stream(argv, _chunks(local))

    def extract_tar(self, local: Path) -> None:
        """Stream a local tarball into `tar xzf - -C /` on the device, with a
        live progress bar — no intermediate copy. Under --dry-run, log and skip."""
        if dry_run.get():
            console.kv("would extract", f"{local.name} -> /")
            return
        argv = ["ssh", *self._ssh_opts(), self.login, "tar xzf - -C /"]
        self._exec.stream(argv, _chunks(local))

    @property
    def reachable(self) -> bool:
        """True if SSH answers. Read-only, safe under --dry-run."""
        try:
            _ = self.read("true")
        except (subprocess.SubprocessError, OSError):
            return False
        return True

    def _board_info(self) -> BoardInfo:
        """``ubus call system board``, parsed and cached (immutable per device)."""
        if self._info is None:
            self._info = msgspec.json.decode(self.read("ubus call system board"), type=BoardInfo)
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

    def log_window(self, path: RemotePath) -> "LogWindow":
        """Stamp `path` and return a handle onto what follows the stamp."""
        nonce = uuid.uuid4().hex
        _ = self.run(f"echo '{_WINDOW_MARK} {nonce}' >> {path}")
        return LogWindow(self, path, nonce)


@dataclass
class LogWindow:
    """A device log bounded to what was written while the window was open.

    Device logs outlive the runs they are read for.
    An unbounded read therefore takes an earlier run's lines for this one's.
    The stamp written on open is the lower bound; closing captures the upper one.

    `read` is always stamp-relative; `with` matters only for the upper bound.
    """

    _dev: "Device"
    path: RemotePath
    nonce: str
    _closed: str | None = None

    def read(self) -> str:
        """Everything after the stamp.

        A missing stamp means the log rotated or was truncated,
        so this aborts rather than return lines that predate the window.
        """
        text = self._dev.read(f"sed -n '/{_WINDOW_MARK} {self.nonce}/,$p' {self.path}")
        require(
            self.nonce in text,
            f"the window marker is gone from {self.path} — it rotated or was "
            + "truncated, so the lines still there may predate this run; "
            + "refusing to report rather than risk stale evidence",
        )
        return text

    def __enter__(self) -> "LogWindow":
        return self

    def __exit__(self, exc_type: object, exc: object, tb: object) -> bool:
        # An in-flight exception is the more useful failure;
        # capturing here would bury it behind a marker-missing abort.
        if exc_type is None:
            self._closed = self.read()
        return False

    @property
    def text(self) -> str:
        """What the window captured when it closed."""
        if self._closed is None:
            msg = "log window is still open — read() inside the block, or use text after it"
            raise RuntimeError(msg)
        return self._closed


def _chunks(local: Path) -> Iterator[bytes]:
    """Yield the file in chunks, advancing a byte-count progress bar as each is sent."""
    with local.open("rb") as f, console.progress(local.name, local.stat().st_size) as advance:
        while chunk := f.read(_CHUNK):
            yield chunk
            advance(len(chunk))

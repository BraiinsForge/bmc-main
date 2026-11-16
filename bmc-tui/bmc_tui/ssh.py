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

"""One SSH connection per login for the whole run, shared by every ssh it makes.

The first ssh to a login keeps its connection open behind a socket in a private directory,
and every later ssh to it goes through that socket.
A reboot ends the connection, so the next ssh asks for the password again.
`nix copy` gets the socket through `NIX_SSHOPTS`, ahead of the user's own,
and upstream Nix joins the connection, unless the user's own options carry a `-S`.
Determinate Nix 3.21.2 and later log in on their own: they run its ssh with `-S none`,
and wait for a `LocalCommand` that a client of a shared connection never runs.
The options go on the command line, which takes precedence over the user's ssh config.

`for_the_run` opens the set for a run and closes it however the run ends (see `exits`).
Outside one there is no set, and every ssh makes its own connection.
"""

import shutil
import socket
import subprocess
import tempfile
import threading
from collections.abc import Generator
from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass, field
from enum import Enum, auto
from pathlib import Path
from typing import Final, Protocol, cast

from bmc_tui import exits


class Runner(Protocol):
    """Runs a control command to completion, raising when it fails."""

    def run(self, argv: list[str], /) -> "subprocess.CompletedProcess[str]": ...


_SOCKET_ROOT = "/tmp"
_CONTROL_TIMEOUT_S = 10.0
_PROBE_TIMEOUT_S = 1.0
# Outlasts a long build between two steps,
# and bounds a root session left by a killed run
_IDLE_CONNECTION_LIMIT = "1h"


class _TimedRunner:
    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        # A stuck master must not hold up the cleanups after it
        return subprocess.run(
            argv,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            check=True,
            timeout=_CONTROL_TIMEOUT_S,
        )


class ControlError(Exception):
    """An `ssh -O` command to a shared connection failed, or did not do what it said."""


def _abandoned(path: Path) -> bool:
    """Whether nothing listens on the socket any more.
    Asked only of a socket whose master already failed to answer `ssh -O`."""
    try:
        with socket.socket(socket.AF_UNIX) as probe:
            # A stopped master's full backlog would otherwise block the connect for good
            probe.settimeout(_PROBE_TIMEOUT_S)
            probe.connect(str(path))
    except (ConnectionRefusedError, FileNotFoundError):
        return True
    except OSError:
        return False
    return False


@dataclass(frozen=True)
class _Unopened:
    pass


@dataclass
class _Open:
    root: Path
    sockets: dict[str, Path] = field(default_factory=dict)


@dataclass(frozen=True)
class _Closed:
    pass


class SharedConnections:
    """The run's connections, one socket per login in a private directory."""

    def __init__(self, runner: Runner | None = None) -> None:
        self._runner: Runner = runner or _TimedRunner()
        self._state: _Unopened | _Open | _Closed = _Unopened()
        # Worker threads ssh too
        self._lock: threading.Lock = threading.Lock()

    def _socket(self, login: str) -> Path:
        state = self._state
        if isinstance(state, _Closed):
            # Nothing would close a connection opened now
            msg = f"BUG: ssh to {login} after the run's connections were closed"
            raise RuntimeError(msg)
        if isinstance(state, _Unopened):
            # Not under TMPDIR: each nested `nix develop` adds 17 characters to it,
            # and two take a socket path past macOS's 104 bytes
            state = _Open(Path(tempfile.mkdtemp(prefix="deck-ssh-", dir=_SOCKET_ROOT)))
            self._state = state
        # Named here rather than by ssh's `%C`, as checking a socket takes its real path
        return state.sockets.setdefault(login, state.root / str(len(state.sockets)))

    def _known_sockets(self) -> dict[str, Path]:
        return self._state.sockets if isinstance(self._state, _Open) else {}

    def options(self, login: str) -> list[str]:
        with self._lock:
            path = self._socket(login)
        return [
            "-o",
            "ControlMaster=auto",
            "-o",
            f"ControlPath={path}",
            "-o",
            f"ControlPersist={_IDLE_CONNECTION_LIMIT}",
        ]

    def join_options(self, login: str) -> list[str]:
        """Options for an ssh that only joins the login's connection.
        It logs in on its own when no master runs, and never leaves one behind."""
        with self._lock:
            path = self._socket(login)
        # Spelled out, as the user's ssh config may set ControlMaster
        return ["-o", "ControlMaster=no", "-o", f"ControlPath={path}"]

    def _control(self, path: Path, login: str, *command: str) -> None:
        try:
            _ = self._runner.run(["ssh", "-o", f"ControlPath={path}", "-O", *command, login])
        except subprocess.CalledProcessError as e:
            stderr = cast("str | None", e.stderr) or ""
            why = f"exit {e.returncode}: {stderr.strip()}"
        except subprocess.TimeoutExpired as e:
            why = f"no answer within {e.timeout:.0f}s"
        except OSError as e:
            why = str(e)
        else:
            return
        msg = f"`ssh -O {command[0]}` to {login} failed ({why})"
        raise ControlError(msg)

    def _running(self, path: Path, login: str) -> bool:
        """Whether a master holds the socket, raising if one is there but does not answer."""
        if not path.exists():
            return False
        try:
            self._control(path, login, "check")
        except ControlError:
            # A killed master leaves its socket behind.
            # Left in place, as the next ssh replaces it,
            # and removing it could race that ssh.
            if not _abandoned(path):
                raise
            return False
        return True

    def add_forward(self, login: str, spec: str) -> None:
        """Have the login's connection listen on the `-L spec` forward until `cancel_forward`.
        Raises rather than falling back to a connection of its own, as an `ssh -L` would."""
        with self._lock:
            path = self._socket(login)
        self._control(path, login, "forward", "-L", spec)

    def cancel_forward(self, login: str, spec: str) -> None:
        """Remove the `-L spec` forward the login's connection holds."""
        with self._lock:
            path = self._known_sockets().get(login)
        if path is None or not self._running(path, login):
            return
        try:
            self._control(path, login, "cancel", "-L", spec)
        except ControlError:
            # The master can end in between, taking the forward with it
            if not _abandoned(path):
                raise

    def _end_each(self, sockets: list[tuple[str, Path]]) -> list[str]:
        """End each connection, returning what went wrong with each that does not end."""
        failures: list[str] = []
        for login, path in sockets:
            # No `-O exit` after a failed check, as a stuck master hangs on that too
            try:
                if self._running(path, login):
                    self._control(path, login, "exit")
            except ControlError as e:
                # A master going down with its device can end between the check and the exit
                if not _abandoned(path):
                    failures.append(str(e))
        return failures

    def end_all(self) -> None:
        """End every connection now, keeping the sockets' directory.
        The next ssh to a login opens a new one.
        Goes on past a connection that does not end, then raises one error naming each such."""
        with self._lock:
            sockets = sorted(self._known_sockets().items())
        failures = self._end_each(sockets)
        if failures:
            raise ControlError("; ".join(failures))

    def close(self) -> None:
        """End each connection and remove the sockets.

        Connections that do not end keep their sockets, and the error says how to end them by hand.
        The set is closed either way, so a second call does nothing.
        """
        with self._lock:
            state = self._state
            self._state = _Closed()
            if not isinstance(state, _Open):
                return
            failures = self._end_each(sorted(state.sockets.items()))
            if failures:
                # The master's command line holds its socket path, on Linux as its process title,
                # and a stopped master holds SIGTERM until it is continued.
                # The bracket keeps the pattern off the shell that runs it,
                # and the trailing slash off that shell's `rm -r` line.
                # One command per line, so each can be pasted on its own.
                kill = f"pkill -KILL -f '[/]{str(state.root)[1:]}/'"
                remove = f"rm -r '{state.root}'"
                msg = f"{'; '.join(failures)}. Still open, these end them:\n{kill}\n{remove}"
                raise RuntimeError(msg)
            shutil.rmtree(state.root, ignore_errors=True)


connections: ContextVar[SharedConnections | None] = ContextVar("ssh_connections", default=None)


class FromTheRun(Enum):
    """Stands for whichever set `ssh.connections` holds, the run's own or none outside a run."""

    CONNECTIONS = auto()


FROM_THE_RUN: Final = FromTheRun.CONNECTIONS


@contextmanager
def for_the_run(runner: Runner | None = None) -> Generator[None]:
    """Share one connection per login across the block,
    closed when the enclosing `trapped` run ends."""
    shared = SharedConnections(runner)
    exits.on_exit("closing the shared SSH connections", shared.close)
    token = connections.set(shared)
    try:
        yield
    finally:
        connections.reset(token)

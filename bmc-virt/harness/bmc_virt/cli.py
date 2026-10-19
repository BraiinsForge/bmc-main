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

"""CLI entry points for bmc-virt.

Provides subcommands for interacting with a running VM:
  bmc-virt ssh -- <command>
  bmc-virt pull <remote> <local>
  bmc-virt push <local> <remote>
  bmc-virt events [--raw]
  bmc-virt wait <event>
  bmc-virt exec -- <command>
"""

import sys
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Annotated

import tyro
from rich.console import Console
from tyro.conf import Positional, arg, subcommand

from bmc_virt import ui
from bmc_virt.events import Event
from bmc_virt.ssh import Ssh
from bmc_virt.vm import VM

_console = Console(stderr=True)
_out = Console()


@dataclass(frozen=True)
class Connection:
    host: str
    event_port: int
    ssh_port: int

    def ssh(self) -> Ssh:
        return Ssh(host=self.host, port=self.ssh_port)

    def vm(self) -> VM:
        return VM.connect(host=self.host, event_port=self.event_port, ssh_port=self.ssh_port)


@dataclass
class SshCommand:
    """Run a command on the VM via SSH."""

    cmd: Annotated[Positional[tuple[str, ...]], arg(metavar="CMD")] = ()  # command to run

    def run(self, conn: Connection) -> None:
        result = conn.ssh().run(" ".join(self.cmd), check=False)
        if result.stdout:
            sys.stdout.write(result.stdout)
        if result.stderr:
            sys.stderr.write(result.stderr)
        sys.exit(result.returncode)


@dataclass
class PullCommand:
    """Copy file from VM to host."""

    remote: Annotated[Positional[str], arg(metavar="REMOTE")]  # remote path on VM
    local: Annotated[Positional[str], arg(metavar="LOCAL")]  # local destination path

    def run(self, conn: Connection) -> None:
        conn.ssh().pull(remote=self.remote, local=self.local)
        _console.print(f"[green]Pulled[/green] {self.remote} → {self.local}")


@dataclass
class PushCommand:
    """Copy file from host to VM."""

    local: Annotated[Positional[str], arg(metavar="LOCAL")]  # local source path
    remote: Annotated[Positional[str], arg(metavar="REMOTE")]  # remote path on VM

    def run(self, conn: Connection) -> None:
        conn.ssh().push(local=self.local, remote=self.remote)
        _console.print(f"[green]Pushed[/green] {self.local} → {self.remote}")


@dataclass
class EventsCommand:
    """Stream events from the VM."""

    raw: bool = False  # output raw JSONL

    def run(self, conn: Connection) -> None:
        vm = conn.vm()
        try:
            for evt in vm.events():
                if self.raw:
                    data = {"name": evt.name, "ts": evt.ts.isoformat(), "data": evt.data}
                    _out.print_json(data=data)
                else:
                    ui.event(evt)
        except KeyboardInterrupt:
            pass
        finally:
            vm.close()


@dataclass
class WaitCommand:
    """Wait for an event."""

    event: Annotated[Positional[str], arg(metavar="EVENT")]  # event name (e.g. app.ready)
    timeout: float = 60  # timeout in seconds

    def run(self, conn: Connection) -> None:
        try:
            event = Event(self.event)
        except ValueError:
            _console.print(f"[red]Unknown event:[/red] {self.event}")
            _console.print(f"[dim]Available: {', '.join(e.value for e in Event)}[/dim]")
            sys.exit(1)

        vm = conn.vm()
        try:
            evt = vm.wait_for(event, timeout=self.timeout)
            _out.print_json(data={"name": evt.name, "ts": evt.ts.isoformat(), "data": evt.data})
        finally:
            vm.close()


@dataclass
class ExecCommand:
    """Execute a shell command on the VM via event daemon."""

    cmd: Annotated[Positional[tuple[str, ...]], arg(metavar="CMD")] = ()  # shell command to run
    timeout: float = 30  # timeout in seconds

    def run(self, conn: Connection) -> None:
        vm = conn.vm()
        try:
            result = vm.exec(" ".join(self.cmd), timeout=self.timeout)
            _out.print_json(
                data={
                    "id": result.id,
                    "ok": result.ok,
                    "data": result.data,
                    "error": result.error,
                }
            )
        finally:
            vm.close()


type Command = (
    Annotated[SshCommand, subcommand("ssh")]
    | Annotated[PullCommand, subcommand("pull")]
    | Annotated[PushCommand, subcommand("push")]
    | Annotated[EventsCommand, subcommand("events")]
    | Annotated[WaitCommand, subcommand("wait")]
    | Annotated[ExecCommand, subcommand("exec")]
)


def dispatch(
    command: Command,
    host: str = "localhost",
    event_port: int = 5920,
    ssh_port: int = 2222,
) -> None:
    """BMC virtual machine harness.

    Args:
        command: what to do on the VM
        host: VM host
        event_port: event daemon port
        ssh_port: SSH port
    """
    command.run(Connection(host=host, event_port=event_port, ssh_port=ssh_port))


def main(argv: Sequence[str] | None = None) -> None:
    """CLI entry point."""
    tyro.cli(dispatch, args=argv, prog="bmc-virt", config=(tyro.conf.OmitSubcommandPrefixes,))


if __name__ == "__main__":
    main()

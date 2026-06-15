"""VM event/command rendering.

The generic presentation (headers, status, panels, prompts) lives in
``bmc_tui.console`` and is re-exported here so existing ``bmc_virt.ui``
consumers keep importing it from this module unchanged. This file adds only
the renderers bound to the VM protocol (``commands``/``events``).
"""

from rich.text import Text

from bmc_tui import console as _c
from bmc_tui.console import *  # noqa: F403  re-export generic presentation
from bmc_virt.commands import Ack, Cmd
from bmc_virt.events import ReceivedEvent


def event(evt: ReceivedEvent) -> None:
    """Pretty-print a single event."""
    ts = Text(_c.format_ts(evt.ts), style="dim")
    name = _c.styled_name(evt.name)
    parts: list[Text | str] = [ts, name]

    if evt.data:
        parts.append(_c.styled_data(evt.data))

    _c._print(*parts)


def event_raw(evt: ReceivedEvent) -> None:
    """Print an event as JSONL."""
    _c.out.print_json(data={"name": evt.name, "ts": evt.ts.isoformat(), "data": evt.data})


def cmd_sent(command: Cmd, **data: object) -> None:
    """Print a command being sent (like zx $.verbose)."""
    # For shell.exec, show `$ <command>` like a terminal
    if command == Cmd.SHELL_EXEC and "cmd" in data:
        _c._print(f"[dim]$[/dim] [bold]{data['cmd']}[/bold]")
        return
    # For other commands: `$ service.restart name="d-bmc-virt-relay"`
    line = Text("$ ", style="dim")
    line.append_text(_c.styled_name(command))
    if data:
        line.append(" ")
        line.append_text(_c.styled_data(data))
    _c._print(line)


def ack_result(result: Ack) -> None:
    """Pretty-print a command ack — output first, then status."""
    has_output = result.data.get("stdout") or result.data.get("stderr")
    if has_output:
        if result.data.get("stdout"):
            for line in result.data["stdout"].rstrip().splitlines():
                _c._print(f"[dim]│[/dim] {line}")
        if result.data.get("stderr"):
            for line in result.data["stderr"].rstrip().splitlines():
                _c._print(f"[dim]│[/dim] [yellow]{line}[/yellow]")
    else:
        _c._print("[dim]│ (no output)[/dim]")
    if result.ok:
        _c._print("[bold green]OK[/bold green]")
    else:
        _c._print("[bold red]FAIL[/bold red]")
        if result.error:
            _c._print(f"  {result.error}", style="red")
    _c._print()

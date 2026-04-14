"""Rich-based output formatting — errors, warnings, event streaming."""

from datetime import datetime
from typing import Any

from rich.console import Console
from rich.panel import Panel
from rich.status import Status
from rich.syntax import Syntax
from rich.text import Text

from bmc_virt.commands import Ack, Cmd
from bmc_virt.events import ReceivedEvent

console = Console(stderr=True)
out = Console()

# Blank line collapsing — tracks consecutive blank prints
_blank_count = 0
_MAX_BLANK = 2


def _print(*args: object, style: str | None = None) -> None:
    """Print with blank line collapsing."""
    global _blank_count  # noqa: PLW0603
    is_blank = not args or (len(args) == 1 and args[0] == "")
    if is_blank:
        _blank_count += 1
        if _blank_count <= _MAX_BLANK:
            out.print()
    elif style:
        _blank_count = 0
        out.print(*args, style=style)
    else:
        _blank_count = 0
        out.print(*args)


def format_ts(ts: datetime) -> str:
    """Format a datetime to human-readable DD/MM/YYYY HH:mm:ss."""
    return ts.astimezone().strftime("%d/%m/%Y %H:%M:%S")


def header(title: str) -> None:
    """Print a section header with vertical spacing."""
    _print()
    _print()
    out.rule(f"[bold]{title}[/bold]", style="cyan", align="left")
    _print()


def styled_name(dotted: str) -> Text:
    """Style a dotted name like object.property — namespace dim, leaf bold cyan."""
    result = Text()
    namespace, _, leaf = dotted.rpartition(".")

    if namespace:
        result.append(namespace, style="bright")
        result.append(".", style="dim")
    result.append(leaf, style="bold cyan")

    return result


def _styled_data(data: dict[str, Any]) -> Text:
    """Style a data dict like IDE semantic highlighting: dim keys, colored values."""
    result = Text()
    for i, (k, v) in enumerate(data.items()):
        if i > 0:
            result.append(" ", style="dim")
        result.append(f"{k}", style="dim italic")
        result.append("=", style="dim")
        if isinstance(v, str):
            result.append(f'"{v}"', style="green")
        elif isinstance(v, bool):
            result.append(str(v).lower(), style="magenta")
        elif isinstance(v, int | float):
            result.append(str(v), style="yellow")
        else:
            result.append(str(v), style="white")
    return result


def event(evt: ReceivedEvent) -> None:
    """Pretty-print a single event."""
    ts = Text(format_ts(evt.ts), style="dim")
    name = styled_name(evt.name)
    parts: list[Text | str] = [ts, name]

    if evt.data:
        parts.append(_styled_data(evt.data))

    _print(*parts)


def event_raw(evt: ReceivedEvent) -> None:
    """Print an event as JSONL."""
    out.print_json(data={"name": evt.name, "ts": evt.ts.isoformat(), "data": evt.data})


def cmd_sent(command: Cmd, **data: Any) -> None:
    """Print a command being sent (like zx $.verbose)."""
    # For shell.exec, show `$ <command>` like a terminal
    if command == Cmd.SHELL_EXEC and "cmd" in data:
        _print(f"[dim]$[/dim] [bold]{data['cmd']}[/bold]")
        return
    # For other commands: `$ service.restart name="d-bmc-virt-relay"`
    line = Text("$ ", style="dim")
    line.append_text(styled_name(command))
    if data:
        line.append(" ")
        line.append_text(_styled_data(data))
    _print(line)


def ack_result(result: Ack) -> None:
    """Pretty-print a command ack — output first, then status."""
    has_output = result.data.get("stdout") or result.data.get("stderr")
    if has_output:
        if result.data.get("stdout"):
            for line in result.data["stdout"].rstrip().splitlines():
                _print(f"[dim]│[/dim] {line}")
        if result.data.get("stderr"):
            for line in result.data["stderr"].rstrip().splitlines():
                _print(f"[dim]│[/dim] [yellow]{line}[/yellow]")
    else:
        _print("[dim]│ (no output)[/dim]")
    if result.ok:
        _print("[bold green]OK[/bold green]")
    else:
        _print("[bold red]FAIL[/bold red]")
        if result.error:
            _print(f"  {result.error}", style="red")
    _print()


def ok(msg: str) -> None:
    """Print a success message."""
    _print(f"[green]✓[/green] {msg}")


def warn(msg: str) -> None:
    """Print a warning."""
    console.print(f"[yellow]⚠[/yellow] {msg}")


def error(msg: str) -> None:
    """Print an error."""
    console.print(f"[red]✗[/red] {msg}")


def kv(key: str, value: str) -> None:
    """Print a key-value pair."""
    _print(f"[dim]{key}:[/dim] {value}")


def cmd_output(text: str) -> None:
    """Print command output (indented, dimmed)."""
    for line in text.rstrip().splitlines():
        _print(f" [dim]{line}[/dim]")


def spinner(msg: str) -> Status:
    """Context manager that shows a dots spinner with a message.

    Usage::

        with ui.spinner("pulling trace..."):
            vm.rr.pull(path)
    """
    return out.status(f"[dim]{msg}[/dim]", spinner="dots")


def panel(
    text: str,
    *,
    title: str,
    style: str = "cyan",
    lexer: str | None = None,
) -> None:
    """Print text in a bordered panel with a title.

    With ``lexer``, the content is syntax-highlighted::

        ui.panel("sudo modprobe msr", title="To fix", style="yellow", lexer="bash")
    """
    content: str | Syntax = (
        Syntax(text, lexer, theme="monokai", background_color="default") if lexer else text
    )
    out.print(Panel(content, title=title, border_style=style, padding=(1, 2)))


def code(text: str, lexer: str = "text") -> None:
    """Print syntax-highlighted code block.

    Usage::

        ui.code(dump_output, lexer="text")
    """
    out.print(Syntax(text.rstrip(), lexer, theme="monokai"))

"""Rich-based output formatting — headers, status, panels, user prompts."""

import sys
from datetime import datetime
from typing import Protocol

from rich.console import Console
from rich.markup import escape
from rich.panel import Panel
from rich.status import Status
from rich.syntax import Syntax
from rich.text import Text

__all__ = [
    "INSTRUCT_HINT",
    "SupportsCapture",
    "cmd_output",
    "code",
    "console",
    "error",
    "format_ts",
    "header",
    "human_size",
    "instruct_user",
    "kv",
    "lit",
    "ok",
    "out",
    "panel",
    "spinner",
    "styled_data",
    "styled_name",
    "warn",
]

INSTRUCT_HINT = "Press Enter when done"


class SupportsCapture(Protocol):
    """Anything with a ``.capture(label)`` method.

    Lets ``instruct_user`` accept a metrics collector without importing a
    concrete collector type (which would create a circular dep in callers
    that already import this module).
    """

    def capture(self, label: str = ...) -> object: ...


console = Console(stderr=True)
out = Console()

# Blank line collapsing — tracks consecutive blank prints
_blank_count = 0
_MAX_BLANK = 2

# One consistent style for literal values (paths, versions, hosts, sizes) —
# the inline-code look, instead of rich's piecemeal auto-highlighting.
_LITERAL = "magenta"


def _print(*args: object, style: str | None = None, highlight: bool = True) -> None:
    """Print with blank line collapsing."""
    global _blank_count  # noqa: PLW0603
    is_blank = not args or (len(args) == 1 and args[0] == "")
    if is_blank:
        _blank_count += 1
        if _blank_count <= _MAX_BLANK:
            out.print()
    else:
        _blank_count = 0
        out.print(*args, style=style, highlight=highlight)


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


def styled_data(data: dict[str, object]) -> Text:
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


def ok(msg: str) -> None:
    """Print a success message (literals in `msg` may use console.lit)."""
    _print(f"[green]✓[/green] {msg}", highlight=False)


def warn(msg: str) -> None:
    """Print a warning."""
    console.print(f"[yellow]⚠[/yellow] {msg}", highlight=False)


def error(msg: str) -> None:
    """Print an error (literals in `msg` may use console.lit)."""
    console.print(f"[red]✗[/red] {msg}", highlight=False)


def kv(key: str, value: str) -> None:
    """Print a key-value pair; the value is styled as a literal."""
    _print(Text.assemble((f"{key}: ", "dim"), (value, _LITERAL)))


def lit(value: object) -> str:
    """Markup for a literal value (path, version, host, size) — inline-code look.

    Use inside a message string, e.g. ``f"{lit(version)} flashed"``; render it
    with `ok`/`error` (which disable auto-highlighting so the style is uniform).
    """
    return f"[{_LITERAL}]{escape(str(value))}[/{_LITERAL}]"


_SIZE_STEP = 1024


def human_size(num_bytes: int) -> str:
    """Human-readable byte size, e.g. ``42.5 MiB``."""
    size = float(num_bytes)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if size < _SIZE_STEP or unit == "TiB":
            return f"{size:.1f} {unit}"
        size /= _SIZE_STEP
    return f"{size:.1f} TiB"


def cmd_output(text: str) -> None:
    """Print command output (indented, dimmed)."""
    for line in text.rstrip().splitlines():
        _print(f" [dim]{line}[/dim]")


def spinner(msg: str) -> Status:
    """Context manager that shows a dots spinner with a message.

    Usage::

        with console.spinner("pulling trace..."):
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

        console.panel("sudo modprobe msr", title="To fix", style="yellow", lexer="bash")
    """
    content: str | Syntax = (
        Syntax(text, lexer, theme="monokai", background_color="default") if lexer else text
    )
    out.print(Panel(content, title=title, border_style=style, padding=(1, 2)))


def code(text: str, lexer: str = "text") -> None:
    """Print syntax-highlighted code block.

    Usage::

        console.code(dump_output, lexer="text")
    """
    out.print(Syntax(text.rstrip(), lexer, theme="monokai"))


def instruct_user(
    message: str,
    *,
    metrics: SupportsCapture | None = None,
) -> datetime:
    """Print a bold instruction panel and block until the user presses Enter.

    When ``metrics`` is provided, captures two labelled snapshots — one before
    the prompt is shown (``prompt: <message>``) and one after the user acks
    (``ack: <message>``). The pair brackets the user-action window in any
    chart the collector renders.

    Returns the ack timestamp.

    Raises ``RuntimeError`` if stdin is not a TTY — silent continuation in a
    measurement script that depends on a manual gate would produce garbage
    data without warning, so the harness fails loudly instead.
    """
    if not sys.stdin.isatty():
        msg = "instruct_user requires a TTY; refusing to silently skip the manual gate"
        raise RuntimeError(msg)

    if metrics is not None:
        metrics.capture(f">{message}")

    body = Text()
    body.append(message, style="bold")
    body.append("\n\n")
    body.append(INSTRUCT_HINT, style="dim")
    out.print(Panel(body, border_style="yellow", padding=(1, 2)))

    input()
    acked_at = datetime.now().astimezone()

    if metrics is not None:
        metrics.capture(f"<{message}")

    return acked_at

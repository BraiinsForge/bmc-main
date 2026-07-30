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

"""Rich-based output formatting — headers, status, panels, user prompts."""

import shutil
import subprocess
import sys
import time
from collections import deque
from collections.abc import Callable, Generator
from contextlib import contextmanager
from contextvars import ContextVar
from datetime import datetime
from pathlib import Path
from typing import Literal, Protocol

from rich.console import Console, Group, RenderableType
from rich.live import Live
from rich.markup import escape
from rich.panel import Panel
from rich.progress import (
    BarColumn,
    DownloadColumn,
    Progress,
    TaskID,
    TextColumn,
    TimeRemainingColumn,
    TransferSpeedColumn,
)
from rich.prompt import Confirm
from rich.spinner import Spinner
from rich.status import Status
from rich.syntax import Syntax
from rich.text import Text

__all__ = [
    "INSTRUCT_HINT",
    "SupportsCapture",
    "alert",
    "cmd_output",
    "code",
    "confirm",
    "console",
    "desktop_notify",
    "error",
    "format_ts",
    "header",
    "human_size",
    "instruct_user",
    "kv",
    "lit",
    "live_scrollback",
    "mark_run_start",
    "notify",
    "ok",
    "out",
    "panel",
    "progress",
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


Level = Literal["info", "success", "warn", "error"]

APP_NAME = "deck"
_ICON = {
    "info": "dialog-information",
    "success": "emblem-default",
    "warn": "dialog-warning",
    "error": "dialog-error",
}
# Nothing is `low`: GNOME files a low-urgency notification straight into the
# message tray without ever showing a banner, and every caller here has already
# decided the user looked away. The level picks the icon; it must not decide
# whether the notification is seen at all.
_URGENCY = {"info": "normal", "success": "normal", "warn": "normal", "error": "critical"}


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


def blank() -> None:
    """Print a blank line, subject to the same collapsing as any other."""
    _print()


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
    """Print command output (indented, dimmed) to stderr, keeping it on the
    same stream as the error header it explains — a redirect must not split
    them. The output is arbitrary text, so it is escaped — brackets in it must
    not parse as rich markup — and auto-highlighting is off so lines stay
    uniform dim, not piecemeal-styled."""
    for line in text.rstrip().splitlines():
        console.print(f" [dim]{escape(line)}[/dim]", highlight=False)


def spinner(msg: str) -> Status:
    """Context manager that shows a dots spinner with a message.

    Usage::

        with console.spinner("pulling trace..."):
            vm.rr.pull(path)
    """
    return out.status(f"[dim]{msg}[/dim]", spinner="dots")


def countdown(label: str, seconds: int) -> None:
    """Block for `seconds`, ticking the remainder into a spinner label.

    A minute-long wait behind a fixed label reads as a hang.
    """
    with out.status("", spinner="dots") as status:
        for remaining in range(seconds, 0, -1):
            status.update(f"[dim]{label} ({remaining}s)[/dim]")
            time.sleep(1)


@contextmanager
def progress(label: str, total: int) -> Generator[Callable[[int], None], None, None]:
    """Byte-count progress bar; yields an ``advance(n)`` callback.

    Usage::

        with console.progress("firmware.tar", size) as advance:
            for chunk in source:
                send(chunk)
                advance(len(chunk))
    """
    bar = Progress(
        TextColumn("[dim]{task.description}[/dim]"),
        BarColumn(),
        DownloadColumn(),
        TimeRemainingColumn(),
        console=out,
    )
    with bar:
        task = bar.add_task(label, total=total)
        yield lambda n: bar.advance(task, n)


# ── bounded live region ("local scrollback") ─────────────────────────────────

SCROLLBACK_TAIL: int = 20


class LiveScrollback:
    """Handle for a `live_scrollback` region: intent-level updates driving one
    in-place display. It knows nothing of any wire format — the caller translates
    a command's progress into these calls, so a flood collapses in place."""

    def __init__(self, live: Live, log_path: Path, tail: int) -> None:
        self._live = live
        self._log_path = log_path
        self._phase: str | None = None
        self._note: str | None = None
        self._download_bar = Progress(
            TextColumn("[dim]downloading[/dim]"),
            BarColumn(),
            DownloadColumn(),
            TransferSpeedColumn(),
            TimeRemainingColumn(),
            console=out,
        )
        self._download: TaskID | None = None
        self._downloading = False
        self.tail: deque[str] = deque(maxlen=tail)
        self._render()

    def phase(self, text: str) -> None:
        # A phase change ends any download; download() re-arms it if bytes resume.
        self._phase = text
        self._downloading = False
        self._render()

    def download(self, done: int, total: int | None) -> None:
        """Advance the download bar; an unknown `total` leaves it indeterminate."""
        task = self._download
        if task is None:
            task = self._download = self._download_bar.add_task("download", total=total)
        self._download_bar.update(task, completed=done, total=total)
        self._downloading = True
        self._render()

    def realize(self, total_paths: int) -> None:
        self._note = f"realizing {total_paths} store paths"
        self._render()

    def realized(self) -> None:
        """Realization ended. The note outlives its phase otherwise, so a later
        `verifying`/`building` renders beside a line still claiming store paths
        are being realized."""
        self._note = None
        self._render()

    def gc(self, deleted_paths: int, freed_bytes: int | None = None) -> None:
        freed = f", {human_size(freed_bytes)} freed" if freed_bytes is not None else ""
        self._note = f"collected {deleted_paths} paths{freed}"
        self._render()

    def echo(self, line: str) -> None:
        """Append a raw line, keeping only the last `tail` on screen."""
        self.tail.append(line)
        self._render()

    def _render(self) -> None:
        rows: list[RenderableType] = []
        # While bytes flow, the bar's own motion is the activity cue; the spinner
        # shows only between downloads (verifying, building, …).
        if self._downloading and self._download is not None:
            rows.append(self._download_bar.get_renderable())
        elif self._phase is not None:
            rows.append(Spinner("dots", text=Text(self._phase, style="cyan")))
        if self._note is not None:
            rows.append(Text(f"  {self._note}", style="dim"))
        # Text() (not from_markup) so raw device lines with [brackets] stay literal.
        rows.extend(Text(f"  {line}", style="dim") for line in self.tail)
        rows.append(Text(f"see {self._log_path}", style="dim"))
        self._live.update(Group(*rows))


@contextmanager
def live_scrollback(
    *, log_path: Path, window: int | None = None
) -> Generator[LiveScrollback, None, None]:
    """A bounded, in-place live region for a command that would otherwise flood
    the terminal, with the full output referenced at `log_path`. Yields a handle
    the caller drives; `transient`, so scrollback stays clean on exit.

    Usage::

        with console.live_scrollback(log_path=log) as live:
            live.phase("realizing")
            live.download(done, total)
            live.echo(raw_line)
    """
    with Live(console=out, transient=True, refresh_per_second=12) as live:
        yield LiveScrollback(live, log_path, window or SCROLLBACK_TAIL)


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


# ── Run clock + attention notifications ──────────────────────────────────────

_run_started: ContextVar[float | None] = ContextVar("run_started", default=None)


def mark_run_start() -> None:
    """Stamp the run's start time; `alert` uses it to stay quiet on quick runs."""
    _run_started.set(time.monotonic())


def notify(summary: str, *, body: str | None = None, level: Level = "info") -> None:
    """Ring the terminal bell and best-effort fire a desktop notification."""
    if sys.stdout.isatty():
        out.bell()
    desktop_notify(summary, body=body, level=level)


def alert(
    summary: str, *, body: str | None = None, after: float = 10.0, level: Level = "info"
) -> None:
    """Notify that attention is needed, but only when interactive and the run
    has been going at least `after` seconds — a quick run means the user is
    watching, so stay silent (the undistract-me heuristic)."""
    if not sys.stdout.isatty():
        return
    started = _run_started.get()
    if started is not None and time.monotonic() - started < after:
        return
    notify(summary, body=body, level=level)


def confirm(question: str) -> bool:
    """Ask a yes/no question (default no). Returns False when not a TTY; alerts
    before blocking so a looked-away user knows input is needed."""
    if not sys.stdin.isatty():
        return False
    alert("confirmation required", body=question)
    return Confirm.ask(question, default=False, console=out)


def desktop_notify(summary: str, *, body: str | None = None, level: Level = "info") -> None:
    """Best-effort OS notification via whichever notifier is on PATH; silent if none.
    Standalone of the bell, so scripts can fire one without a TTY.

    An error is sent `critical`, which most desktops keep on screen until
    it is dismissed; everything else is `normal`, which still raises a banner.
    """
    text = body or summary
    if shutil.which("notify-send"):
        cmd = [
            "notify-send",
            "--app-name",
            APP_NAME,
            "--urgency",
            _URGENCY[level],
            "--icon",
            _ICON[level],
            summary,
            text,
        ]
    elif shutil.which("terminal-notifier"):
        cmd = ["terminal-notifier", "-title", summary, "-message", text, "-group", APP_NAME]
    elif shutil.which("osascript"):
        title = summary.replace('"', '\\"')
        message = text.replace('"', '\\"')
        cmd = ["osascript", "-e", f'display notification "{message}" with title "{title}"']
    else:
        return
    subprocess.run(cmd, check=False, capture_output=True)

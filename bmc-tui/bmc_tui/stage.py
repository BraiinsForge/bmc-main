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

"""Staged-procedure engine: stages are plain functions guarded by prose verbs.

A stage function runs its guard verbs (`require`, `ensure`, `done_if`) and then
its step, top to bottom. `@stage` reports the outcome; `@entrypoint` parses CLI
args from the script's dataclass parameter (via tyro) and turns an `Abort` into a
clean error plus a non-zero exit.
"""

import functools
import inspect
import shlex
import subprocess
from collections.abc import Callable
from contextvars import ContextVar
from typing import ParamSpec, get_type_hints

import tyro
from rich.traceback import install as install_rich_traceback

from bmc_tui import console

# Set by scripts, consumed by the transport seam: turns mutating commands into
# log-and-skip while read-only probes still run for real.
dry_run: ContextVar[bool] = ContextVar("dry_run", default=False)

_StageParams = ParamSpec("_StageParams")


class Abort(Exception):  # noqa: N818  reads with the verbs; deliberately not *Error
    """A precondition failed that the harness cannot fix.

    Carries a human-actionable hint; rendered as a clean error, never a
    traceback. The hint *is* the remedy the human performs.
    """

    def __init__(self, hint: str) -> None:
        super().__init__(hint)
        self.hint = hint


class _Skip(Exception):  # noqa: N818  internal control-flow signal, not an error
    """Raised by `done_if` to short-circuit a stage whose goal is already met."""


def require(cond: bool, hint: str) -> None:
    """Advisory precondition: if `cond` is false, abort with `hint`."""
    if not cond:
        raise Abort(hint)


def ensure(check: Callable[[], bool], remedy: Callable[[], None], hint: str | None = None) -> None:
    """Auto-remediable precondition: if `check()` is false, run `remedy` and
    re-check. Still false → abort (with `hint` if given)."""
    if check():
        return
    remedy()
    if not check():
        raise Abort(hint or "remedy did not satisfy the precondition")


def done_if(cond: bool) -> None:
    """Idempotency guard: if `cond` is true, skip the rest of the stage."""
    if cond:
        raise _Skip


def stage(
    name: str,
) -> Callable[[Callable[_StageParams, str | None]], Callable[_StageParams, None]]:
    """Mark a function as a stage: print its section header, run it, then show
    its result — the string it returns (or "ok"), "already satisfied" on a
    `done_if` skip, or its failure hint.

    A stage returns a short status line describing what it did; `ParamSpec`
    preserves the wrapped function's typed signature, so call-sites stay
    type-checked.
    """

    def decorate(fn: Callable[_StageParams, str | None]) -> Callable[_StageParams, None]:
        @functools.wraps(fn)
        def run_stage(*args: _StageParams.args, **kwargs: _StageParams.kwargs) -> None:
            console.header(f"{name}  [dim](dry-run)[/dim]" if dry_run.get() else name)
            try:
                result = fn(*args, **kwargs)
            except _Skip:
                console.ok("already satisfied")
            else:
                console.ok(result or "ok")
            # Abort propagates uncaught — the entrypoint renders it once, under
            # this section's header.

        return run_stage

    return decorate


def entrypoint(main: Callable[..., None]) -> Callable[..., None]:
    """Wrap a script's `main`: parse CLI args from its dataclass parameter via
    tyro (auto `--help`, usage-on-error), run it, turn an `Abort` into a rendered
    error plus `exit 1`, and Ctrl-C into a clean exit. A failed subprocess is
    rendered with its captured stdout/stderr — that output is the actual error,
    and the default exception message drops it. Anything unexpected gets a
    readable rich traceback rather than a raw dump. Call-sites stay bare
    top-to-bottom statements.
    """
    install_rich_traceback(show_locals=True, width=120)
    params = list(inspect.signature(main).parameters.values())
    args_type = get_type_hints(main)[params[0].name] if params else None

    @functools.wraps(main)
    def wrapped(argv: list[str] | None = None) -> None:
        console.mark_run_start()
        try:
            if args_type is None:
                main()
            else:
                main(tyro.cli(args_type, args=argv))
        except Abort as e:
            console.error(e.hint)
            raise SystemExit(1) from e
        except subprocess.CalledProcessError as e:
            cmd = e.cmd if isinstance(e.cmd, str) else shlex.join(e.cmd)
            # Negative returncode is how subprocess reports a signal death.
            status = (
                f"killed by signal {-e.returncode}" if e.returncode < 0 else f"exit {e.returncode}"
            )
            console.error(f"{status}: {console.lit(cmd)}")
            for captured in (e.stdout, e.stderr):
                text = _output_text(captured)
                if text.strip():
                    console.cmd_output(text)
            raise SystemExit(1) from e
        except KeyboardInterrupt:
            console.warn("interrupted")
            raise SystemExit(130) from None

    return wrapped


def _output_text(captured: str | bytes | None) -> str:
    """Captured process output as text — `run` captures str, `stream` bytes,
    and either stream may not have been captured at all."""
    if captured is None:
        return ""
    if isinstance(captured, bytes):
        return captured.decode(errors="replace")
    return captured

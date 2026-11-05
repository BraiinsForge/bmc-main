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

"""Render every surface in `en-XA` under `.tmp/intl-sweep/` for a person to judge.

Plain English left in a shot is copy that bypassed the translation pipeline.
Run from the repository root, as `just intl::sweep` does.
"""

import os
import shutil
import time
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from bmc_tui import console
from bmc_tui.process import stream_lines
from bmc_tui.stage import Abort, entrypoint, stage
from playwright.sync_api import Error as PlaywrightError
from web_probe.browser import BrowsersMissing, Unreachable, open_probe
from web_probe.device import (
    LOGIN_PATH,
    EditorUnreached,
    Interrupted,
    LanguageStuck,
    LanguageUnknown,
    LoginRefused,
    LoginStalled,
    SceneLeft,
    SetupLanguageMissing,
    SetupSwitchMissing,
    TabsMissing,
    current_language,
    in_setup,
    log_in,
    offered_languages,
    shoot_login,
    shoot_pages,
    shoot_setup,
    speaking,
)

EXIT_INTERRUPTED = 130
LANGUAGE = "en-XA"
SCRATCH = Path(".tmp")
SWEEP = SCRATCH / "intl-sweep"
LOGS = SWEEP / "logs"
GALLERY = SWEEP / "gallery"
WIDGETS = SWEEP / "widgets"
NO_MARKER = "the device lacks its i18n marker (/etc/bmc/i18n)"
NO_PACK = (
    "the device lacks its pack. Install it with"
    " `nix run .#deck -- deploy --device <ip> --packages language-en-xa`,"
    " then restart bmc (`/etc/init.d/bmc-compositor restart`), which reads packs only"
    " as it starts. On the mock, `just fe::serve` loads it."
)


@dataclass(frozen=True)
class Sweep:
    """Render the overlays, every widget recipe and, given its URL, the web UI in `en-XA`."""

    # Where the device's web UI answers, such as `http://localhost:6070`.
    # Left out, the web is skipped. Nothing is started there, the server is the caller's to run.
    url: str | None = None
    # The device's web password, `root` on the mock.
    password: str = "root"


def widget_recipes(root: Path) -> list[tuple[str, Path]]:
    """Every gallery recipe under `root`, the widget libraries' too, named `<widget>-<recipe>`.
    Each is given whole, as the gallery reads a relative one from its own folder."""
    recipes = (
        recipe
        for recipe in root.glob("**/gallery/*.capture.toml")
        if "target" not in recipe.relative_to(root).parts
    )
    return sorted((_recipe_name(recipe), recipe.resolve()) for recipe in recipes)


def _recipe_name(recipe: Path) -> str:
    return f"{recipe.parent.parent.name}-{recipe.name.removesuffix('.capture.toml')}"


def run_logged(
    argv: list[str], log: Path, phase: str, env: Mapping[str, str] | None = None
) -> None:
    """Run `argv`, its output in a live tail and whole in `log`. A failure aborts at the log."""
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w", encoding="utf-8") as file, console.live_scrollback(log_path=log) as live:
        live.phase(phase)

        def on_line(line: str) -> None:
            _ = file.write(f"{line}\n")
            live.echo(line)

        code = stream_lines(argv, on_line, env=env)

    if code:
        raise Abort(f"{phase} failed (exit {code}): see {console.lit(log)}")


def capture(recipe: str, out: Path, log: Path, phase: str) -> None:
    # The dev shell keeps the machine's GL driver off the loader path,
    # so the gallery finds no EGL device there.
    # Its software rasteriser renders instead, the same shots on any machine,
    # unless the caller sets `GALLERY_SOFTWARE_GL=0` for its own driver, as `wasm::scenes` allows.
    env = {"GALLERY_SOFTWARE_GL": "1", **os.environ}
    # The recipe takes its OUT as a name under `.tmp/`, not as a path.
    name = str(out.relative_to(SCRATCH))
    argv = ["just", "gallery::capture", recipe, name, "--global", f"Language={LANGUAGE}"]
    run_logged(argv, log, phase, env)


@stage("the overlays")
def overlays() -> str:
    with a_step("capturing the overlays' recipe"):
        capture("capture.toml", GALLERY, LOGS / "overlays.log", "capturing the overlays' recipe")
    return f"shots under {console.lit(GALLERY)}"


@stage("the widgets")
def widgets() -> str:
    recipes = widget_recipes(Path("widgets-wasm"))
    if not recipes:
        raise Abort("no `gallery/*.capture.toml` under widgets-wasm/")

    # One broken recipe must not cost the shots of the rest.
    failed: list[str] = []
    for index, (name, recipe) in enumerate(recipes, start=1):
        phase = f"{name} ({index} of {len(recipes)})"
        try:
            with a_step("capturing", name):
                capture(str(recipe), WIDGETS / name, LOGS / f"widget-{name}.log", phase)
        except Abort as failure:
            console.error(failure.hint)
            failed.append(name)

    if failed:
        raise Abort(f"{len(failed)} of {len(recipes)} recipes failed: {', '.join(failed)}")
    return f"{console.lit(len(recipes))} recipes, shots under {console.lit(WIDGETS)}"


@stage("the web")
def web(url: str, password: str) -> str:
    try:
        return _walk_web(url, password)
    except Interrupted as error:
        console.error(console.lit(error))
        raise SystemExit(EXIT_INTERRUPTED) from error
    except BrowsersMissing as error:
        raise Abort(f"{error} - run the sweep as `just intl::sweep`, which sets it") from error
    except Unreachable as error:
        raise Abort(f"{error} - is the web UI running at {url}?") from error
    except LoginRefused as error:
        raise Abort(f"{error} - pass the device's password with --password") from error
    except SetupSwitchMissing as error:
        raise Abort(f"{error}, {NO_MARKER}, or its languages failed to load") from error
    except SetupLanguageMissing as error:
        raise Abort(f"{error}, {NO_PACK}") from error
    except (
        LoginStalled,
        LanguageUnknown,
        LanguageStuck,
        EditorUnreached,
        SceneLeft,
        TabsMissing,
    ) as error:
        raise Abort(str(error)) from error
    except PlaywrightError as error:
        raise Abort(f"the web walk failed: {error.message.splitlines()[0]}") from error


class Steps:
    """A stage's steps: the current one on the live spinner, each ended one left in the output."""

    def __init__(self, phase: Callable[[str], None]) -> None:
        self._phase: Callable[[str], None] = phase
        self._current: str | None = None
        self._started: float = 0.0

    def start(self, name: str, value: str | None = None) -> None:
        """Begin the step `name`, of the exact `value` if it has one.
        The two stay apart as the spinner shows text as it is, and the output styles the value."""
        self.finish()
        self._current = name if value is None else f"{name} {console.lit(value)}"
        self._started = time.monotonic()
        self._phase(name if value is None else f"{name} {value}")

    def finish(self) -> None:
        if self._current is not None:
            console.ok(self._timed())
            self._current = None

    def fail(self) -> None:
        if self._current is not None:
            console.error(self._timed())
            self._current = None

    def _timed(self) -> str:
        return f"{self._current} ({console.lit(f'{time.monotonic() - self._started:.1f} s')})"


@contextmanager
def steps_of(phase: Callable[[str], None]) -> Generator[Steps]:
    steps = Steps(phase)
    try:
        yield steps
    except BaseException:
        steps.fail()
        raise
    steps.finish()


@contextmanager
def a_step(name: str, value: str | None = None) -> Generator[None]:
    """One step around a command that runs its own live display, so it needs no spinner here."""

    def spinner_is_the_commands(_phase: str) -> None:
        pass

    with steps_of(spinner_is_the_commands) as steps:
        steps.start(name, value)
        yield


def _walk_web(url: str, password: str) -> str:
    out = SWEEP / "web"
    log = LOGS / "web.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    with (
        log.open("w", encoding="utf-8") as file,
        console.live_scrollback(log_path=log) as live,
        steps_of(live.phase) as steps,
        open_probe(url) as probe,
    ):

        def shot_taken(path: str, shot: Path) -> None:
            line = f"{path} -> {shot}"
            _ = file.write(f"{line}\n")
            live.echo(line)

        steps.start("opening the web UI")
        if in_setup(probe):
            steps.start("capturing the setup pages")
            setup_shots = 0
            for path, shot in shoot_setup(probe, out, LANGUAGE):
                setup_shots += 1
                shot_taken(path, shot)
            return (
                f"captured {console.lit(setup_shots)} of the setup's views"
                f" under {console.lit(out)}, its forms left unsent,"
                " as sending one would change the device"
            )
        steps.start("logging in")
        log_in(probe, password)

        steps.start("reading the device's languages")
        # Aborted, not skipped: `--url` asked for the web, and a passing run would hide its absence.
        offered = offered_languages(probe)
        if not offered:
            raise Abort(f"no languages offered, {NO_MARKER}")
        if LANGUAGE not in offered:
            raise Abort(f"{LANGUAGE} is not offered, {NO_PACK}")

        # `speaking` leaves such a device alone, so no step may claim to have switched it.
        switches = current_language(probe) != LANGUAGE
        if switches:
            steps.start("switching the device to", LANGUAGE)
        with speaking(probe, LANGUAGE):
            # The side navigation is clicked through before the first page is shot.
            steps.start("finding the pages")
            shots: list[Path] = []
            for path, shot in shoot_pages(probe, out):
                if not shots:
                    steps.start("capturing the pages")
                shots.append(shot)
                shot_taken(path, shot)
            steps.start("capturing the login page")
            if login := shoot_login(probe, out):
                shots.append(login)
                shot_taken(LOGIN_PATH, login)
            if switches:
                steps.start("setting the device's language back")

    return f"{console.lit(len(shots))} shots under {console.lit(out)}"


def main(args: Sweep) -> None:
    # A page gone or a recipe renamed would otherwise leave last run's shots beside this run's.
    shutil.rmtree(SWEEP, ignore_errors=True)
    # First, as the quickest: a server that does not answer is told before the captures, not after.
    if args.url is None:
        console.warn("the web skipped: no --url given")
    else:
        web(args.url, args.password)
    overlays()
    widgets()
    console.ok(f"every shot under {console.lit(SWEEP)}")


if __name__ == "__main__":
    entrypoint(main, name="intl sweep")()

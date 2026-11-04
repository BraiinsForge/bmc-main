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
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

from bmc_tui import console
from bmc_tui.process import stream_lines
from bmc_tui.stage import Abort, entrypoint, stage
from playwright.sync_api import Error as PlaywrightError
from web_probe.browser import BrowsersMissing, Unreachable, open_probe
from web_probe.device import (
    LanguageUnknown,
    LoginRefused,
    LoginStalled,
    TabsMissing,
    in_setup,
    log_in,
    offered_languages,
    shoot_login,
    shoot_pages,
    speaking,
)

LANGUAGE = "en-XA"
SCRATCH = Path(".tmp")
SWEEP = SCRATCH / "intl-sweep"
LOGS = SWEEP / "logs"
GALLERY = SWEEP / "gallery"
WIDGETS = SWEEP / "widgets"


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
            capture(str(recipe), WIDGETS / name, LOGS / f"widget-{name}.log", phase)
        except Abort as failure:
            console.error(failure.hint)
            failed.append(name)

    if failed:
        raise Abort(f"{len(failed)} of {len(recipes)} recipes failed: {', '.join(failed)}")
    return f"{len(recipes)} recipes, shots under {console.lit(WIDGETS)}"


@stage("the web")
def web(url: str, password: str) -> str:
    try:
        return _walk_web(url, password)
    except BrowsersMissing as error:
        raise Abort(f"{error} - run the sweep as `just intl::sweep`, which sets it") from error
    except Unreachable as error:
        raise Abort(f"{error} - is the web UI running at {url}?") from error
    except LoginRefused as error:
        raise Abort(f"{error} - pass the device's password with --password") from error
    except (LoginStalled, LanguageUnknown, TabsMissing) as error:
        raise Abort(str(error)) from error
    except PlaywrightError as error:
        raise Abort(f"the web walk failed: {error.message.splitlines()[0]}") from error


def _walk_web(url: str, password: str) -> str:
    out = SWEEP / "web"
    with open_probe(url) as probe:
        if in_setup(probe):
            shot = probe.shot(out / "setup.png")
            return (
                f"the device is being set up, its first page is at {console.lit(shot)}."
                f" The wizard's steps are not walked, and it speaks English, not {LANGUAGE}"
            )
        log_in(probe, password)

        # Aborted, not skipped: `--url` asked for the web, and a passing run would hide its absence.
        offered = offered_languages(probe)
        if not offered:
            raise Abort("no languages offered, the device lacks its i18n marker (/etc/bmc/i18n)")

        if LANGUAGE not in offered:
            msg = (
                f"{LANGUAGE} is not offered, the device lacks its pack. Install it with"
                " `nix run .#deck -- deploy --device <ip> --packages language-en-xa`,"
                " then restart bmc (`/etc/init.d/bmc-compositor restart`), which reads packs only"
                " as it starts. On the mock, `just fe::serve` loads it."
            )
            raise Abort(msg)

        with speaking(probe, LANGUAGE):
            shots = [shot for _, shot in shoot_pages(probe, out)]
            if login := shoot_login(probe, out, password):
                shots.append(login)

    return f"{len(shots)} shots under {console.lit(out)}"


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
    entrypoint(main)()

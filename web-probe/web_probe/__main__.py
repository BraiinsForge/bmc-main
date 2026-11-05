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

"""Walk the device's web UI: log in, switch language, shoot every page and tab it leads to."""

import sys
from contextlib import nullcontext
from pathlib import Path
from typing import TextIO

import tyro
from bmc_tui import console
from playwright.sync_api import Error as PlaywrightError
from tyro.conf import Positional

from web_probe.browser import BrowsersMissing, Probe, Unreachable, open_probe, read_failure
from web_probe.device import (
    LOGIN_PATH,
    EditorUnreached,
    Interrupted,
    LanguageStuck,
    LanguageUnknown,
    LoginRefused,
    LoginStalled,
    SceneLeft,
    TabsMissing,
    log_in,
    offered_languages,
    shoot_login,
    shoot_pages,
    speaking,
)

SHOTS = Path(".tmp/web-probe")
EXIT_INTERRUPTED = 130


def walk(
    origin: Positional[str],
    password: str = "root",
    language: str | None = None,
    out: Path = SHOTS,
) -> int:
    """Log in, switch language if asked, and shoot every page and tab the UI leads to, whole.

    The language is the device's own setting, so a switched one is switched back after.
    To reach the combined-scene editor, the walk creates a scene on the device and deletes it after.

    Args:
        origin: where the web UI answers, such as `http://localhost:6070`
        password: the device's web password, `root` on the mock
        language: a tag the device offers, such as `en-XA`, or left out for the device's own
        out: where the shots go, one PNG per page or tab
    """
    with open_probe(origin) as probe:
        last_shot: Path | None = None
        try:
            log_in(probe, password)
            offered = offered_languages(probe)
            if offered:
                print(f"offered languages: {', '.join(offered)}")
            else:
                print("no languages offered: the device lacks its i18n marker (/etc/bmc/i18n)")

            if language is not None and language not in offered:
                console.error(f"{console.lit(language)} is not offered, nothing walked")
                return 1

            with nullcontext() if language is None else speaking(probe, language):
                for path, last_shot in shoot_pages(probe, out):
                    print(f"{path} -> {last_shot}")
                if login := shoot_login(probe, out):
                    last_shot = login
                    print(f"{LOGIN_PATH} -> {login}")
        except Interrupted as error:
            console.error(console.lit(error))
            return EXIT_INTERRUPTED
        except (
            PlaywrightError,
            Unreachable,
            LoginRefused,
            LoginStalled,
            LanguageUnknown,
            EditorUnreached,
            SceneLeft,
            TabsMissing,
        ) as error:
            _report_failure(probe, error, last_shot)
            return 1
        _report_console_errors(probe, sys.stdout)

    return 0


def _report_console_errors(probe: Probe, file: TextIO) -> None:
    errors = [entry for entry in probe.console() if entry.kind == "error"]
    print(f"console errors: {len(errors)}", file=file)
    for entry in errors:
        print(f"  {entry.text}", file=file)


def _detail(key: str, value: str) -> None:
    console.console.print(f"[dim]{key}:[/dim] {console.lit(value)}", highlight=False)


def _report_failure(probe: Probe, error: Exception, last_shot: Path | None) -> None:
    """Why the walk stopped and how far it got, in place of a traceback, all on stderr,
    so that a redirect cannot part the failure from its details.
    The page it stopped on is not asked for: setting a switched language back has left it."""
    failure = read_failure(str(error))
    console.error(f"the walk failed: {console.lit(failure.reason)}")
    _detail("last shot", str(last_shot) if last_shot else "none")

    if failure.waiting_for:
        _detail("waiting for", failure.waiting_for)
        _detail("resolved to", failure.resolved_to or "nothing")

    for state in failure.states:
        _detail("state", state)

    for blocker in failure.blocked_by:
        _detail("blocked by", blocker)

    if failure.log and not failure.waiting_for:
        console.cmd_output("\n".join(failure.log))

    _report_console_errors(probe, sys.stderr)


def main() -> None:
    try:
        code = tyro.cli(walk)
    except (BrowsersMissing, LanguageStuck) as error:
        console.error(console.lit(error))
        code = 1
    except PlaywrightError as error:
        # Only the browser's own start and stop fail outside the walk's report.
        console.error(f"the browser failed: {console.lit(read_failure(error.message).reason)}")
        code = 1
    except KeyboardInterrupt:
        console.error("the walk was interrupted")
        code = EXIT_INTERRUPTED

    raise SystemExit(code)


if __name__ == "__main__":
    main()

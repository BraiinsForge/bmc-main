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

"""Walk the device's web UI: log in, switch language, shoot every page it links to."""

from contextlib import nullcontext
from pathlib import Path

import tyro
from tyro.conf import Positional

from web_probe.browser import BrowsersMissing, Unreachable, open_probe
from web_probe.device import (
    LOGIN_PATH,
    LanguageUnknown,
    LoginRefused,
    LoginStalled,
    log_in,
    offered_languages,
    shoot_login,
    shoot_pages,
    speaking,
)

SHOTS = Path(".tmp/web-probe")


def walk(
    origin: Positional[str],
    password: str = "root",
    language: str | None = None,
    out: Path = SHOTS,
) -> int:
    """Log in, switch language if asked, and shoot every page the UI leads to, whole.

    The language is the device's own setting, so a switched one is switched back after.

    Args:
        origin: where the web UI answers, such as `http://localhost:6070`
        password: the device's web password, `root` on the mock
        language: a tag the device offers, such as `en-XA`, or left out for the device's own
        out: where the shots go, one PNG per page
    """
    with open_probe(origin) as probe:
        log_in(probe, password)
        offered = offered_languages(probe)
        if offered:
            print(f"offered languages: {', '.join(offered)}")
        else:
            print("no languages offered: the device lacks its i18n marker (/etc/bmc/i18n)")
        if language is not None and language not in offered:
            print(f"{language} is not offered, nothing walked")
            return 1
        with nullcontext() if language is None else speaking(probe, language):
            for path, shot in shoot_pages(probe, out):
                print(f"{path} -> {shot}")
            if login := shoot_login(probe, out, password):
                print(f"{LOGIN_PATH} -> {login}")
        errors = [entry for entry in probe.console() if entry.kind == "error"]
        print(f"console errors: {len(errors)}")
        for entry in errors:
            print(f"  {entry.text}")
    return 0


def main() -> None:
    try:
        code = tyro.cli(walk)
    except (BrowsersMissing, Unreachable, LoginRefused, LoginStalled, LanguageUnknown) as error:
        print(error)
        code = 1
    raise SystemExit(code)


if __name__ == "__main__":
    main()

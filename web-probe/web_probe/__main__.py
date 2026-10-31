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

from pathlib import Path

import tyro
from tyro.conf import Positional

from web_probe.browser import open_probe
from web_probe.device import current_language, log_in, offered_languages, pages, set_language

SHOTS = Path(".tmp/web-probe")


def shot_name(path: str) -> str:
    """The file a page's shot is saved as: `/display/1` as `display_1.png`, `/` as `home.png`."""
    return f"{path.strip('/').replace('/', '_') or 'home'}.png"


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
        password: the device's web password; the mock's is `root`
        language: a tag the device offers, such as `en-XA`; left out, the device's own language
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
            print(f"{language} is not offered; nothing walked")
            return 1
        before = current_language(probe)
        if language is not None:
            set_language(probe, language)
        try:
            for path in pages(probe):
                print(f"{path} -> {probe.shot(out / shot_name(path))}")
        finally:
            if before is not None and before != language:
                set_language(probe, before)
                print(f"language set back to {before}")
        errors = [entry for entry in probe.console() if entry.kind == "error"]
        print(f"console errors: {len(errors)}")
        for entry in errors:
            print(f"  {entry.text}")
    return 0


def main() -> None:
    raise SystemExit(tyro.cli(walk))


if __name__ == "__main__":
    main()

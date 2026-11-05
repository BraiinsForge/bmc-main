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

"""Unit tests for capturing a device's setup pages without changing its setup."""

from collections.abc import Callable, Generator
from contextlib import contextmanager
from pathlib import Path
from typing import cast

import pytest
from playwright.sync_api import Response

from web_probe import device
from web_probe.browser import Probe
from web_probe.device import SetupLanguageMissing, SetupSwitchMissing, shoot_setup

CONNECT = "/init_connect"
DEVICE = "/init_setup"
OUT = Path("out")
_SCAN_URL = "http://deck/braiins.bmc.web.InitialSetupService/ScanWifi"


class Answered:
    """A response, as far as the walk reads one."""

    def __init__(self, url: str) -> None:
        self.url: str = url


class Located:
    def __init__(self, done: list[str], selector: str, count: int) -> None:
        self._done: list[str] = done
        self._selector: str = selector
        self._count: int = count

    @property
    def first(self) -> "Located":
        return self

    def count(self) -> int:
        return self._count

    def click(self) -> None:
        self._done.append(f"click {self._selector}")

    def wait_for(self) -> None:
        self._done.append(f"wait for {self._selector}")


class Setup:
    """A probe on a device in setup, keeping what was done to it in order."""

    def __init__(
        self,
        monkeypatch: pytest.MonkeyPatch,
        path: str,
        *,
        language: str | None = "en-US",
        shot_fails: bool = False,
        networks: int = 2,
    ) -> None:
        self.done: list[str] = []
        self._networks: int = networks
        self.page: Setup = self
        self.viewport_size: dict[str, int] = {"width": 1_280, "height": 800}
        self.language: str | None = language
        self._path: str = path
        self._shot_fails: bool = shot_fails
        monkeypatch.setattr(device, "_setup_language", self.current)
        monkeypatch.setattr(device, "_setup_languages", self.offered)
        monkeypatch.setattr(device, "_set_setup_language", self.pick)

    def current(self, _probe: Probe) -> str | None:
        return self.language

    def offered(self, _probe: Probe) -> list[str]:
        return ["en-US", "en-XA"]

    def pick(self, _probe: Probe, tag: str) -> None:
        self.language = tag
        self.done.append(f"speak {tag}")

    def set_viewport_size(self, size: dict[str, int]) -> None:
        self.viewport_size = size

    def visit(self, _path: str) -> None:
        pass

    def path(self) -> str:
        return self._path

    def shot(self, file: Path) -> Path:
        if self._shot_fails:
            raise RuntimeError("the shot failed")
        assert self.viewport_size["height"] > 800, "at 800 px the form scrolls out of the shot"
        self.done.append(f"shot {file.name} in {self.language}")
        return file

    def click(self, selector: str) -> None:
        self.done.append(f"click {selector}")

    def locator(self, selector: str) -> Located:
        return Located(self.done, selector, self._networks)

    @contextmanager
    def expect_response(self, matcher: Callable[[Response], bool]) -> Generator[None]:
        answer = cast(Response, cast(object, Answered(_SCAN_URL)))
        assert matcher(answer), "the wait has to recognise the device's real answer"
        self.done.append("expect the scan")
        yield


def walked(setup: Setup, tag: str = "en-XA") -> list[str]:
    return [path for path, _ in shoot_setup(cast(Probe, cast(object, setup)), OUT, tag)]


NETWORKS = 'wait for #initial-setup-wifi-networks[aria-busy="false"]'


def test_the_wifi_setup_is_captured_with_both_its_forms_and_sends_neither(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    setup = Setup(monkeypatch, CONNECT)

    assert walked(setup) == [
        CONNECT,
        f"{CONNECT}#wifi",
        f"{CONNECT}#wifi-password",
        f"{CONNECT}#wifi-manual",
    ]
    assert setup.done == [
        "speak en-XA",
        "shot init_connect.png in en-XA",
        "expect the scan",
        "click #initial-setup-welcome-continue",
        NETWORKS,
        "shot init_connect_wifi.png in en-XA",
        'click #initial-setup-wifi-networks [role~="button"]',
        "wait for #initial-setup-wifi-selected-network-dropdown",
        "shot init_connect_wifi-password.png in en-XA",
        "click #initial-setup-wifi-back",
        NETWORKS,
        "click #initial-setup-wifi-other-networks",
        "wait for #initial-setup-wifi-manual-ssid",
        "shot init_connect_wifi-manual.png in en-XA",
        "click #initial-setup-wifi-back",
        NETWORKS,
        "speak en-US",
    ]


def test_a_wifi_setup_listing_no_network_still_captures_its_manual_form_and_says_so(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    setup = Setup(monkeypatch, CONNECT, networks=0)

    assert walked(setup) == [CONNECT, f"{CONNECT}#wifi", f"{CONNECT}#wifi-manual"]
    assert "listed no network" in capsys.readouterr().err


def test_the_device_setup_is_captured_and_nothing_on_it_is_clicked(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    setup = Setup(monkeypatch, DEVICE)

    assert walked(setup) == [DEVICE]
    assert setup.done == ["speak en-XA", "shot init_setup.png in en-XA", "speak en-US"]


def test_a_setup_without_a_language_switch_is_refused_before_any_shot(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    setup = Setup(monkeypatch, DEVICE, language=None)

    with pytest.raises(SetupSwitchMissing):
        _ = walked(setup)

    assert setup.done == []


def test_a_setup_not_offering_the_language_is_refused_before_any_shot(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    setup = Setup(monkeypatch, DEVICE)

    with pytest.raises(SetupLanguageMissing, match="does not offer zh"):
        _ = walked(setup, tag="zh")

    assert setup.done == []


def test_a_failure_on_the_setup_pages_still_sets_the_language_back(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    setup = Setup(monkeypatch, DEVICE, shot_fails=True)

    with pytest.raises(RuntimeError, match="the shot failed"):
        _ = walked(setup)

    assert setup.done == ["speak en-XA", "speak en-US"]


def test_a_device_speaking_the_language_is_not_switched_or_set_back(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    setup = Setup(monkeypatch, DEVICE, language="en-XA")

    assert walked(setup) == [DEVICE]
    assert setup.done == ["shot init_setup.png in en-XA"]

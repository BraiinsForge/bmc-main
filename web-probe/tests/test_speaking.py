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

"""Unit tests for switching the device's language around a block."""

from typing import cast

import pytest
from playwright.sync_api import Error as PlaywrightError

from web_probe import device
from web_probe.browser import Probe
from web_probe.device import Interrupted, LanguageStuck, LanguageUnknown, speaking

# `speaking` reaches the page only through the picker calls `Picker` stands in for.
PROBE = cast(Probe, object())


class Picker:
    """The device's language setting, as the language picker reads and sets it."""

    def __init__(
        self,
        language: str | None,
        *,
        setting_back_fails: bool = False,
        setting_back_interrupted: bool = False,
        switch_times_out: bool = False,
    ) -> None:
        self.language: str | None = language
        self.picked: list[str] = []
        self._setting_back_fails: bool = setting_back_fails
        self._setting_back_interrupted: bool = setting_back_interrupted
        self._switch_times_out: bool = switch_times_out

    def current(self, _probe: Probe) -> str | None:
        return self.language

    def pick(self, _probe: Probe, tag: str) -> None:
        if self._setting_back_interrupted and self.picked:
            raise KeyboardInterrupt
        if self._setting_back_fails and self.picked:
            msg = "the picker is gone"
            raise PlaywrightError(msg)
        first = not self.picked
        self.picked.append(tag)
        self.language = tag
        if self._switch_times_out and first:
            msg = "the page did not speak it in time"
            raise PlaywrightError(msg)


def installed(monkeypatch: pytest.MonkeyPatch, picker: Picker) -> Picker:
    monkeypatch.setattr(device, "current_language", picker.current)
    monkeypatch.setattr(device, "set_language", picker.pick)
    return picker


def test_the_device_speaks_its_own_language_again_after_the_block(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    picker = installed(monkeypatch, Picker("en-US"))

    with speaking(PROBE, "en-XA"):
        assert picker.language == "en-XA"

    assert picker.language == "en-US"


def test_a_failing_block_still_sets_the_language_back(monkeypatch: pytest.MonkeyPatch) -> None:
    picker = installed(monkeypatch, Picker("en-US"))

    with pytest.raises(RuntimeError, match="the walk failed"), speaking(PROBE, "en-XA"):
        raise RuntimeError("the walk failed")

    assert picker.language == "en-US"


def test_a_switch_that_times_out_after_it_took_still_sets_the_language_back(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    picker = installed(monkeypatch, Picker("en-US", switch_times_out=True))

    with pytest.raises(PlaywrightError, match="in time"), speaking(PROBE, "en-XA"):
        pass

    assert picker.picked == ["en-XA", "en-US"]


def test_a_device_already_speaking_the_language_is_neither_switched_nor_set_back(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    picker = installed(monkeypatch, Picker("en-XA"))

    with speaking(PROBE, "en-XA"):
        pass

    assert picker.picked == []


def test_a_language_that_could_not_be_set_back_is_refused_before_any_switch(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    picker = installed(monkeypatch, Picker(None))

    with pytest.raises(LanguageUnknown), speaking(PROBE, "en-XA"):
        pass

    assert picker.picked == []


def test_a_failure_setting_the_language_back_leaves_the_blocks_own_error_and_says_so(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    _ = installed(monkeypatch, Picker("en-US", setting_back_fails=True))

    with pytest.raises(RuntimeError, match="the walk failed"), speaking(PROBE, "en-XA"):
        raise RuntimeError("the walk failed")

    assert "language back to en-US failed" in capsys.readouterr().err


def test_a_failure_setting_the_language_back_after_a_clean_block_says_what_failed_and_why(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _ = installed(monkeypatch, Picker("en-US", setting_back_fails=True))

    with (
        pytest.raises(LanguageStuck, match=r"language back to en-US failed: the picker is gone"),
        speaking(PROBE, "en-XA"),
    ):
        pass


def test_an_interrupt_setting_the_language_back_after_a_clean_block_says_so(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _ = installed(monkeypatch, Picker("en-US", setting_back_interrupted=True))

    with (
        pytest.raises(Interrupted, match="interrupted while setting the device's language back"),
        speaking(PROBE, "en-XA"),
    ):
        pass


def test_a_second_interrupt_setting_the_language_back_says_so(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _ = installed(monkeypatch, Picker("en-US", setting_back_interrupted=True))

    with (
        pytest.raises(Interrupted, match="interrupted while setting the device's language back"),
        speaking(PROBE, "en-XA"),
    ):
        raise KeyboardInterrupt


def test_an_interrupt_in_the_block_with_the_language_set_back_stays_plain(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    picker = installed(monkeypatch, Picker("en-US"))

    with pytest.raises(KeyboardInterrupt), speaking(PROBE, "en-XA"):
        raise KeyboardInterrupt

    assert picker.language == "en-US"

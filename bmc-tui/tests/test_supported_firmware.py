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

"""Unit tests for refusing firmware older than the BMC application supports."""

import subprocess
from collections.abc import Callable, Iterable

import pytest
from rich.text import Text

from bmc_tui import catalog, console, nix, supported_firmware
from bmc_tui.device import Device
from bmc_tui.procedures import deploy, init
from bmc_tui.stage import Abort, dry_run
from tests import stubs

_BMC100 = "stm32mp157c-ii3-bmc1"
_BMM101 = "stm32mp157c-ii2-bmm1"
_BMM100 = "stm32mp157c-ii1-am2"


class _Files:
    """Fake Exec answering `cat` of the device files given.

    A missing file reads as empty only behind a `|| true` fallback, as on the device;
    any other command fails.
    """

    def __init__(self, files: dict[str, str]) -> None:
        self._files: dict[str, str] = files

    def run(self, argv: list[str], /) -> "subprocess.CompletedProcess[str]":
        command = argv[-1]
        for path, content in self._files.items():
            if command.startswith(f"cat {path} "):
                return subprocess.CompletedProcess(argv, 0, stdout=f"{content}\n", stderr="")
        if command.startswith("cat ") and command.endswith(" || true"):
            return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
        raise subprocess.CalledProcessError(1, argv)

    def stream(self, argv: list[str], _chunks: Iterable[bytes], /) -> None:
        raise AssertionError(f"the firmware check must not upload: {argv}")

    def stream_output(self, argv: list[str], _on_line: Callable[[str], None], /) -> int:
        raise AssertionError(f"the firmware check must not stream: {argv}")


def _device(platform: str | None, bos_version: str | None) -> Device:
    files = {
        path: content
        for path, content in (
            (supported_firmware.PLATFORM_PATH, platform),
            (supported_firmware.VERSION_PATH, bos_version),
        )
        if content is not None
    }
    return Device("h", backend=_Files(files))


def _check(platform: str | None, bos_version: str | None) -> None:
    supported_firmware.ensure_supported_firmware(_device(platform, bos_version))


def test_bmc100_refuses_the_release_before_26_07() -> None:
    with pytest.raises(Abort) as refused:
        _check(_BMC100, "2026-06-30-0-06ba61b5-26.06")
    names = f"firmware {console.lit('26.06')} is older than {console.lit('26.07')}"
    assert names in str(refused.value), "the refusal names the installed and the minimum version"


def test_bmc100_refuses_a_later_month_of_an_earlier_year() -> None:
    with pytest.raises(Abort, match="older than"):
        _check(_BMC100, "2025-12-15-0-06ba61b5-25.12")


def test_bmc100_refuses_a_release_candidate_of_26_06() -> None:
    with pytest.raises(Abort, match="older than"):
        _check(_BMC100, "2026-06-20-0-06ba61b5-26.06-rc")


def test_bmc100_refuses_a_patch_release_of_26_06() -> None:
    with pytest.raises(Abort, match="older than"):
        _check(_BMC100, "2026-07-02-0-06ba61b5-26.06.1")


def test_refusal_tells_the_user_to_upgrade_the_firmware() -> None:
    with pytest.raises(
        Abort, match=r"upgrade the firmware first, in the device's web UI or with `deck sysupgrade`"
    ):
        _check(_BMC100, "2026-06-30-0-06ba61b5-26.06")


def test_refusal_tells_a_bmm101_owner_to_pass_the_image() -> None:
    # `deck sysupgrade` offers only BMC100 images unless given one.
    with pytest.raises(Abort, match=r"a BMM101 needs its image passed with `--image`"):
        _check(_BMM101, "2026-08-31-0-06ba61b5-26.08")


def test_bmc100_passes_on_exactly_26_07() -> None:
    _check(_BMC100, "2026-07-31-0-06ba61b5-26.07")


def test_bmc100_passes_on_an_earlier_month_of_a_later_year() -> None:
    _check(_BMC100, "2027-01-15-0-06ba61b5-27.01")


@pytest.mark.parametrize(
    "bos_version",
    [
        "2026-08-05-0-06ba61b5-26.07.1",
        "2026-07-31-0-06ba61b5-26.07-plus-nightly",
        "2026-07-21-0-34648a21-26.07-rc",
    ],
    ids=["patch", "plus-nightly", "rc-from-the-feeds"],
)
def test_bmc100_passes_on_builds_of_26_07(bos_version: str) -> None:
    _check(_BMC100, bos_version)


def test_bmm101_is_held_to_its_own_minimum() -> None:
    # 26.08 passes the BMC100 minimum, so a mixed-up product would let it through.
    with pytest.raises(Abort, match="older than"):
        _check(_BMM101, "2026-08-31-0-06ba61b5-26.08-plus")


def test_bmm101_passes_on_exactly_26_09() -> None:
    _check(_BMM101, "2026-09-30-0-06ba61b5-26.09")


def test_an_unchecked_product_passes_without_a_version_file() -> None:
    _check(_BMM100, None)


def test_a_device_without_a_platform_file_is_not_checked() -> None:
    _check(None, None)


def test_a_checked_product_without_a_version_file_names_the_file() -> None:
    with pytest.raises(Abort) as refused:
        _check(_BMC100, None)
    names = f"{console.lit(supported_firmware.VERSION_PATH)} is missing or empty"
    assert names in str(refused.value), "the error names the file to fix"


def test_a_checked_product_with_an_empty_version_file_names_the_file() -> None:
    with pytest.raises(Abort) as refused:
        _check(_BMC100, "")
    names = f"{console.lit(supported_firmware.VERSION_PATH)} is missing or empty"
    assert names in str(refused.value), "an empty file is reported as such, not as unreadable"


def test_a_checked_product_with_an_unparseable_version_names_the_file() -> None:
    with pytest.raises(Abort) as refused:
        _check(_BMC100, "26.07")
    names = f"cannot parse the firmware version in {console.lit(supported_firmware.VERSION_PATH)}"
    assert names in str(refused.value), "the error names the file to fix"


def test_an_unparseable_version_with_markup_still_renders_as_the_error() -> None:
    with pytest.raises(Abort) as refused:
        _check(_BMC100, "garbage [/x]")
    rendered = Text.from_markup(str(refused.value)).plain
    assert "garbage [/x]" in rendered, "the file's text is shown as written, not parsed as markup"


def test_the_check_runs_under_dry_run() -> None:
    token = dry_run.set(True)
    try:
        with pytest.raises(Abort, match="older than"):
            _check(_BMC100, "2026-06-30-0-06ba61b5-26.06")
    finally:
        dry_run.reset(token)


_INIT_STAGES = (
    "ensure_device_reachable",
    "build_init_tarball",
    "build_nix_cli",
    "push_nix_cli",
    "prepare_data_partition",
    "ensure_store_absent",
    "push_init_tarball",
    "run_cli_init",
    "activate_profile",
    "cleanup_remote_artifacts",
)


@pytest.mark.parametrize("dry", [False, True], ids=["real", "dry-run"])
def test_init_refuses_unsupported_firmware_before_its_first_change(
    monkeypatch: pytest.MonkeyPatch, *, dry: bool
) -> None:
    monkeypatch.setattr(console, "header", stubs.returning(None))
    calls: list[str] = []
    for name in _INIT_STAGES:
        monkeypatch.setattr(catalog, name, stubs.recording(calls, name))
    token = dry_run.set(False)
    try:
        with pytest.raises(Abort, match="older than"):
            init.Init(device="h", dry_run=dry).run(
                dev=_device(_BMC100, "2026-06-30-0-06ba61b5-26.06"),
                backend=nix.real(),
            )
    finally:
        dry_run.reset(token)
    assert calls == ["ensure_device_reachable"], "nothing may run after the refusal"


@pytest.mark.parametrize("dry", [False, True], ids=["real", "dry-run"])
def test_deploy_refuses_unsupported_firmware_before_its_first_change(
    monkeypatch: pytest.MonkeyPatch, *, dry: bool
) -> None:
    monkeypatch.setattr(
        deploy, "Device", stubs.returning(_device(_BMC100, "2026-06-30-0-06ba61b5-26.06"))
    )
    monkeypatch.setattr(console, "header", stubs.returning(None))
    monkeypatch.setattr(nix, "real", stubs.returning(object()))
    calls: list[str] = []
    for name in (
        "ensure_grpcurl",
        "ensure_device_reachable",
        "disable_auto_upgrade",
        "ensure_nix_cli",
    ):
        monkeypatch.setattr(catalog, name, stubs.recording(calls, name))

    token = dry_run.set(False)
    try:
        with pytest.raises(Abort, match="older than"):
            deploy.Deploy(device="deck", dry_run=dry).run()
    finally:
        dry_run.reset(token)
    assert calls == ["ensure_grpcurl", "ensure_device_reachable"], (
        "automatic upgrades must stay on and nothing may be copied after the refusal"
    )

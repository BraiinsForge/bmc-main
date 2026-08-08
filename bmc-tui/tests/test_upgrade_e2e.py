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

"""The package harnesses must hand the device back as they found it. A run
disables every production server for the rig's benefit and trusts the rig's
signing key, so one that keeps its registration keeps the device from ever
seeing a real upgrade again."""

import pytest

from bmc_tui import catalog, nix
from bmc_tui.procedures.install_widget_e2e import InstallWidgetE2e
from bmc_tui.procedures.upgrade_e2e import UpgradeE2e
from bmc_tui.stage import Abort


def _stub_run(monkeypatch: pytest.MonkeyPatch, events: list[str], *, fail_at: str | None) -> None:
    """Neutralise every stage the run touches, recording the order."""

    def record(name: str):
        def hook(*_args: object, **_kwargs: object) -> None:
            events.append(name)
            if name == fail_at:
                raise Abort(f"scripted failure in {name}")

        return hook

    for name in (
        "ensure_grpcurl",
        "ensure_device_reachable",
        "ensure_nix_cli",
        "resolve_packages",
        "build_packages",
        "snapshot_profile",
        "require_unclaimed_package_registry",
        "capture_server_registry",
        "capture_nix_conf",
        "start_upgrade_server",
        "register_upgrade_server",
        "require_exclusive_package_server",
        "grpc_login",
        "check_for_upgrade",
        "run_upgrade",
        "verify_profile_advanced",
        "restore_server_registry",
        "restore_nix_conf",
        "stop_upgrade_server",
    ):
        monkeypatch.setattr(catalog, name, record(name))

    monkeypatch.setattr(nix, "real", lambda **_kw: object())
    monkeypatch.setattr(catalog, "package_prefix", lambda _profile: "")


def test_registry_is_restored_after_a_successful_run(monkeypatch: pytest.MonkeyPatch) -> None:
    events: list[str] = []
    _stub_run(monkeypatch, events, fail_at=None)

    UpgradeE2e(device="deck.local", packages=["core"]).run()

    assert events.index("capture_server_registry") < events.index("register_upgrade_server"), (
        "the pre-run registry must be captured before the rig overwrites it"
    )
    assert events.index("capture_nix_conf") < events.index("register_upgrade_server"), (
        "registration adds the rig's substituter and trusted key, so nix.conf "
        "must be captured before it, not after"
    )
    assert events[-3:] == [
        "restore_server_registry",
        "restore_nix_conf",
        "stop_upgrade_server",
    ]


def test_registry_is_restored_when_the_run_fails(monkeypatch: pytest.MonkeyPatch) -> None:
    events: list[str] = []
    _stub_run(monkeypatch, events, fail_at="run_upgrade")

    with pytest.raises(Abort, match="scripted failure in run_upgrade"):
        UpgradeE2e(device="deck.local", packages=["core"]).run()

    assert "restore_server_registry" in events, (
        "a failed run must not strand the device with every production server disabled"
    )
    assert "restore_nix_conf" in events, "nor with a standing trust grant for the rig's signing key"
    assert "stop_upgrade_server" in events


def test_a_restore_failure_does_not_mask_the_primary_failure(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    events: list[str] = []
    _stub_run(monkeypatch, events, fail_at="run_upgrade")

    def failing_restore(*_args: object) -> None:
        events.append("restore_server_registry")
        raise Abort("restore blew up")

    monkeypatch.setattr(catalog, "restore_server_registry", failing_restore)

    with pytest.raises(Abort, match="scripted failure in run_upgrade"):
        UpgradeE2e(device="deck.local", packages=["core"]).run()

    assert "stop_upgrade_server" in events, "the server must still be stopped"


def test_a_restore_failure_fails_an_otherwise_successful_run(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    events: list[str] = []
    _stub_run(monkeypatch, events, fail_at=None)

    def failing_restore(*_args: object) -> None:
        events.append("restore_server_registry")
        raise Abort("restore blew up")

    monkeypatch.setattr(catalog, "restore_server_registry", failing_restore)

    with pytest.raises(Abort, match="restore blew up"):
        UpgradeE2e(device="deck.local", packages=["core"]).run()

    assert "restore_nix_conf" in events, (
        "a device half-restored is the state the session exists to prevent, "
        "so one restore blowing up must not skip the next"
    )
    assert "stop_upgrade_server" in events


def test_the_widget_install_harness_restores_through_the_same_session(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Both package harnesses disable the device's production servers, so
    both must hand the registry back — the guarantee lives in the shared
    session, and this pins that install-widget still goes through it."""
    events: list[str] = []
    _stub_run(monkeypatch, events, fail_at="run_upgrade")
    for name in (
        "remove_package",
        "list_installable_widgets",
        "check_for_install",
        "verify_widget_installed",
    ):
        monkeypatch.setattr(catalog, name, lambda *_a, **_k: None)

    with pytest.raises(Abort, match="scripted failure in run_upgrade"):
        InstallWidgetE2e(device="deck.local", packages=["core"]).run()

    assert "restore_server_registry" in events
    assert "stop_upgrade_server" in events

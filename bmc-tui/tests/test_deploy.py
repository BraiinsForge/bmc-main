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

"""Unit tests for ordinary package deployment choreography."""

from typing import Any

import pytest

from bmc_tui import catalog
from bmc_tui.procedures import deploy


class _Device:
    def __init__(self, _host: str) -> None:
        pass

    def print(self) -> None:
        pass


def test_deploy_observes_activation_without_hard_restart(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(deploy, "Device", _Device)
    monkeypatch.setattr(deploy.console, "header", lambda _title: None)
    monkeypatch.setattr(deploy.nix, "real", lambda **_kwargs: object())
    for name in (
        "ensure_device_reachable",
        "ensure_nix_cli",
        "resolve_packages",
        "build_packages",
        "copy_closures",
        "remove_superseded_packages",
        "register_packages",
        "clear_upgrade_servers",
    ):
        monkeypatch.setattr(deploy.catalog, name, lambda *_args, **_kwargs: None)

    old_pid = catalog.Pid("111")
    observed: list[tuple[Any, catalog.Pid | None]] = []
    monkeypatch.setattr(deploy.catalog, "compositor_pid", lambda _dev: old_pid)
    monkeypatch.setattr(
        deploy.catalog,
        "await_package_activation",
        lambda dev, *, old_pid: observed.append((dev, old_pid)),
    )
    monkeypatch.setattr(
        deploy.catalog,
        "restart_compositor",
        lambda *_args, **_kwargs: pytest.fail("ordinary deploy must not hard-restart"),
    )

    deploy.Deploy(device="deck").run()

    assert len(observed) == 1
    assert observed[0][1] == old_pid

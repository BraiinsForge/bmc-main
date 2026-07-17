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

"""Structure tests for the fault-injection suite."""

import json
import types
from pathlib import Path
from typing import cast

from bmc_tui.device import Device
from bmc_tui.procedures import e2e_sysupgrade_faults as faults
from bmc_tui.procedures.e2e_sysupgrade_faults import E2eSysupgradeFaults
from bmc_tui.stage import dry_run
from tests.test_catalog import _TARGET, _cp, _e2e_image, _e2e_nix, _Exec, _local_server, _Respond


def test_suite_order_is_pinned() -> None:
    assert faults.SUITE_ORDER == (
        "unsigned-feed",
        "untrusted-key-name",
        "wrong-key-signature",
        "corrupt-tarball",
        "download-stall",
        "good-init",
        "store-remnants",
        "missing-store-db",
        "unmounted-store",
        "blank-data-partition",
        "corrupt-fs-metadata",
        "unreachable-rig",
        "malformed-index",
        "wrong-cache-key",
        "cache-swap-retry",
        "same-version-reflash",
    )


def test_every_scenario_id_has_a_driver() -> None:
    ids = {
        "wrong-key-signature",
        "unsigned-feed",
        "untrusted-key-name",
        "corrupt-tarball",
        "download-stall",
        "blank-data-partition",
        "corrupt-fs-metadata",
        "store-remnants",
        "missing-store-db",
        "unmounted-store",
        "cache-swap-retry",
        "unreachable-rig",
        "malformed-index",
        "wrong-cache-key",
        "stale-next-marker",
        "same-version-reflash",
        "shm-local-file",
        "staged-once",
        "servers-json",
    }
    groups = {"a", "b", "c", "d", "all"}
    assert ids | groups <= set(faults._DRIVERS)


def test_suite_order_ids_are_driven() -> None:
    assert all(sid in faults._DRIVERS or sid == "good-init" for sid in faults.SUITE_ORDER)


def test_a_group_runs_scenarios_in_pinned_order(monkeypatch) -> None:
    calls: list[str] = []
    for sid in (
        "wrong-key-signature",
        "unsigned-feed",
        "untrusted-key-name",
        "corrupt-tarball",
        "download-stall",
    ):
        monkeypatch.setitem(faults._DRIVERS, sid, lambda _ctx, sid=sid: calls.append(sid))
    monkeypatch.setattr(faults, "_flash_good_init", lambda _ctx: calls.append("finale"))
    monkeypatch.setattr(faults.catalog, "clear_nix_store", lambda *_a, **_k: calls.append("clear"))
    # SimpleNamespace duck-types the three fields _group_a reads; cast keeps
    # the type checker happy without building a full _Ctx.
    fake_ctx = cast(
        "faults._Ctx",
        types.SimpleNamespace(dev=None, yes=True, run=types.SimpleNamespace(device_mutated=False)),
    )
    faults._group_a(fake_ctx)
    assert calls == [
        "clear",
        "unsigned-feed",
        "untrusted-key-name",
        "wrong-key-signature",
        "corrupt-tarball",
        "download-stall",
        "finale",
    ]


def _dry_routes(sha_a: str) -> _Respond:
    """Read-only replies for the unsigned-feed dry-run: a valid board, ample
    RAM, a reachable rig (non-zero preflight bytes), the verified upload sha,
    and — via the empty default — an absent store and download artifact."""
    board = json.dumps({"board_name": "b", "release": {"target": _TARGET}})
    outputs = {
        "ubus call system board": board,
        "MemAvailable": "999999999",
        "dd bs=64": "64",
        "sha256sum": sha_a,
    }

    def respond(argv: list[str]):
        cmd = argv[-1]
        for key, value in outputs.items():
            if key in cmd:
                return _cp(argv, value)
        return _cp(argv)

    return respond


def test_faults_dry_run_touches_nothing(tmp_path: Path) -> None:
    image_a = _e2e_image(tmp_path, name="a.tar", version="va")
    image_b = _e2e_image(tmp_path, name="b.tar", version="vb")
    exc = _Exec(_dry_routes(image_a.sha256))
    dev = Device("127.0.0.1", backend=exc)
    args = E2eSysupgradeFaults(
        device="127.0.0.1",
        image_a=image_a.path,
        image_b=image_b.path,
        scenario="unsigned-feed",
        serve_ip="127.0.0.1",
        yes=True,
        dry_run=True,
    )
    token = dry_run.set(False)  # the procedure sets it from its own flag
    try:
        args.run(
            dev=dev,
            backend=_e2e_nix(tmp_path),
            make_device=lambda _h: dev,
            make_server=_local_server,
        )
    finally:
        dry_run.reset(token)
    cmds = [argv[-1] for argv in exc.runs]
    assert not exc.streams  # no uploads happened
    assert not any(c.startswith("sysupgrade ") for c in cmds)
    assert not any("rm -rf" in c or "umount" in c or c.startswith("rm -f") for c in cmds)

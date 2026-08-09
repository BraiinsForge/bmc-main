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
import subprocess
import types
from pathlib import Path
from typing import cast

import pytest

from bmc_tui import cli
from bmc_tui.device import Device
from bmc_tui.procedures import e2e_sysupgrade_faults as faults
from bmc_tui.procedures.e2e_sysupgrade_faults import E2eSysupgradeFaults
from bmc_tui.stage import Abort, dry_run
from tests.test_catalog import (
    _TARGET,
    _cp,
    _e2e_image,
    _e2e_nix,
    _Exec,
    _image,
    _local_server,
    _Respond,
)


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
        "full-store",
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
        "full-store",
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
    # The fake finale mimics the real contract: _flash_good_init closes the
    # quiesced window on its way out (after the reboot revives mDNS).
    monkeypatch.setattr(
        faults,
        "_flash_good_init",
        lambda ctx: (calls.append("finale"), setattr(ctx, "quiesced_pin", None)),
    )
    # Pin must happen BEFORE the cleardown: the cleardown's quiesce kills the
    # mDNS name, so the whole group runs on the numeric handle pinned first.
    monkeypatch.setattr(faults.catalog, "pin_device_address", lambda *_a, **_k: calls.append("pin"))
    monkeypatch.setattr(faults, "_pinned", lambda ctx: ctx.dev)
    monkeypatch.setattr(faults.catalog, "clear_nix_store", lambda *_a, **_k: calls.append("clear"))
    # SimpleNamespace duck-types the fields _group_a reads; cast keeps the type
    # checker happy without building a full _Ctx.
    fake_ctx = cast(
        "faults._Ctx",
        types.SimpleNamespace(
            dev=object(),
            yes=True,
            run=types.SimpleNamespace(device_mutated=False),
            quiesced_pin=None,
        ),
    )
    faults._group_a(fake_ctx)
    assert calls == [
        "pin",
        "clear",
        "unsigned-feed",
        "untrusted-key-name",
        "wrong-key-signature",
        "corrupt-tarball",
        "download-stall",
        "finale",
    ]
    assert fake_ctx.quiesced_pin is None  # the window is closed after the group


def test_a_group_failure_leaves_the_quiesced_pin_for_cleanup(monkeypatch) -> None:
    """A failure inside the quiesced window (avahi stopped, mDNS dead until
    the finale's reboot) must leave the numeric pin set: the outer cleanup —
    the servers.json restore included — runs on it, and against the dead
    name it would degrade to 'cleanup failed' exactly when it matters."""
    monkeypatch.setattr(faults.catalog, "pin_device_address", lambda *_a, **_k: None)
    pinned = object()
    monkeypatch.setattr(faults, "_pinned", lambda _ctx: pinned)

    def dying_cleardown(*_a, **_k) -> None:
        raise Abort("cleardown died")

    monkeypatch.setattr(faults.catalog, "clear_nix_store", dying_cleardown)
    fake_ctx = cast(
        "faults._Ctx",
        types.SimpleNamespace(
            dev=object(),
            yes=True,
            run=types.SimpleNamespace(device_mutated=False),
            quiesced_pin=None,
        ),
    )
    with pytest.raises(Abort, match="cleardown died"):
        faults._group_a(fake_ctx)
    assert fake_ctx.quiesced_pin is pinned  # the outer cleanup can still reach the device


def test_b_group_order_and_recovery_wrapper(monkeypatch) -> None:
    calls: list[str] = []
    for sid in (
        "blank-data-partition",
        "corrupt-fs-metadata",
        "store-remnants",
        "missing-store-db",
        "unmounted-store",
    ):
        monkeypatch.setitem(faults._DRIVERS, sid, lambda _ctx, sid=sid: calls.append(sid))
    faults._group_b(cast("faults._Ctx", types.SimpleNamespace()))
    assert calls == [
        "store-remnants",
        "missing-store-db",
        "unmounted-store",
        "blank-data-partition",
        "corrupt-fs-metadata",
    ]


def test_b_recovery_attempts_one_reflash_then_reraises(monkeypatch) -> None:
    recovered: list[str] = []
    monkeypatch.setattr(faults, "_flash_good_init", lambda _ctx: recovered.append("reflash"))

    def failing(_ctx) -> None:
        raise Abort("corrupt-fs-metadata did not repair")

    with pytest.raises(Abort, match="did not repair"):
        faults._with_b_recovery(cast("faults._Ctx", _b_recovery_ctx()), failing)
    assert recovered == ["reflash"]


def test_b_recovery_covers_operational_failures(monkeypatch) -> None:
    """dd/debugfs/umount/flash failures surface as CalledProcessError,
    not Abort — they must trigger the same one-shot recovery."""
    recovered: list[str] = []
    monkeypatch.setattr(faults, "_flash_good_init", lambda _ctx: recovered.append("reflash"))

    def failing(_ctx) -> None:
        raise subprocess.CalledProcessError(1, ["dd"])

    with pytest.raises(subprocess.CalledProcessError):
        faults._with_b_recovery(cast("faults._Ctx", _b_recovery_ctx()), failing)
    assert recovered == ["reflash"]


def test_b_recovery_failure_names_the_manual_fallback(monkeypatch) -> None:
    def broken_reflash(_ctx) -> None:
        raise Abort("device gone")

    monkeypatch.setattr(faults, "_flash_good_init", broken_reflash)

    def failing(_ctx) -> None:
        raise Abort("original")

    ctx = _b_recovery_ctx()
    with pytest.raises(Abort, match="init --wipe"):
        faults._with_b_recovery(cast("faults._Ctx", ctx), failing)
    # A failed recovery leaves the device quiesced with mDNS dead — the pin
    # must survive so the outer cleanup can still reach it.
    assert ctx.quiesced_pin is not None


def _b_recovery_ctx() -> types.SimpleNamespace:
    """A ctx with just the fields _with_b_recovery reads: _pinned needs the
    dev host and the recorded pin, plus a settable quiesced_pin slot."""
    return types.SimpleNamespace(
        dev=types.SimpleNamespace(host="deck.local"),
        run=types.SimpleNamespace(pinned_host=None),
        quiesced_pin=None,
    )


def test_b_recovery_reuses_the_already_pinned_handle(monkeypatch) -> None:
    """Recovery fires with the device quiesced (mDNS possibly dead), so the
    recovery flash must reuse the numeric handle the failed scenario already
    pinned rather than re-resolve the name. _with_b_recovery hands
    _flash_good_init a live quiesced_pin; the finale closes the window
    itself once the reboot revives mDNS (mimicked by the fake)."""
    pinned = object()
    seen: list[object] = []
    monkeypatch.setattr(faults, "_pinned", lambda _ctx: pinned)
    monkeypatch.setattr(
        faults,
        "_flash_good_init",
        lambda ctx: (seen.append(ctx.quiesced_pin), setattr(ctx, "quiesced_pin", None)),
    )
    ctx = types.SimpleNamespace(
        dev=types.SimpleNamespace(host="deck.local"),
        run=types.SimpleNamespace(pinned_host="10.0.0.5"),
        quiesced_pin=None,
    )

    def failing(_ctx) -> None:
        raise Abort("blank-data-partition did not repair")

    with pytest.raises(Abort, match="did not repair"):
        faults._with_b_recovery(cast("faults._Ctx", ctx), failing)
    assert seen == [pinned]  # the recovery flash saw the pin, not a dead name
    assert ctx.quiesced_pin is None  # window closed after recovery


def _dry_routes(sha_a: str) -> _Respond:
    """Read-only replies for the unsigned-feed dry-run: a valid board, ample
    RAM, a reachable rig (non-zero preflight bytes), the verified upload sha,
    and — via the empty default — an absent store and download artifact."""
    board = json.dumps({"board_name": "b", "release": {"target": _TARGET}})
    outputs = {
        "if [ -e /etc/nix-upgrade/servers.json ]": "ABSENT",
        "if [ -e /etc/nix/nix.conf ]": "ABSENT",
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


def _staged_dry_routes(sha_a: str) -> _Respond:
    """Read-only replies for a same-version-reflash dry-run: as _dry_routes,
    plus an initialized store and a running image A so the scenario reaches
    require_staged_once (which unsigned-feed never does)."""
    board = json.dumps({"board_name": "b", "release": {"target": _TARGET}})
    outputs = {
        "if [ -e /etc/nix-upgrade/servers.json ]": "ABSENT",
        "if [ -e /etc/nix/nix.conf ]": "ABSENT",
        "ubus call system board": board,
        "MemAvailable": "999999999",
        "dd bs=64": "64",
        "sha256sum": sha_a,
        "-d /mnt/data/nix": "yes",  # the store is initialized
        "readlink -f /nix/var/nix/gcroots/profiles/bmc/current": "1-link",
        "cat /etc/bos_version": "va",  # running image A
    }

    def respond(argv: list[str]):
        cmd = argv[-1]
        for key, value in outputs.items():
            if key in cmd:
                return _cp(argv, value)
        return _cp(argv)

    return respond


def test_staged_once_dry_run_skips_the_capture_check(tmp_path: Path) -> None:
    """Under --dry-run flash_e2e logs-and-skips, so no flash output is
    captured; require_staged_once must early-return instead of raising its
    BUG guard. The unsigned-feed dry-run never reaches that stage — a
    same-version reflash does, so this is where the crash would surface."""
    image_a = _e2e_image(tmp_path, name="a.tar", version="va")
    image_b = _e2e_image(tmp_path, name="b.tar", version="vb")
    exc = _Exec(_staged_dry_routes(image_a.sha256))
    dev = Device("127.0.0.1", backend=exc)
    args = E2eSysupgradeFaults(
        device="127.0.0.1",
        image_a=image_a.path,
        image_b=image_b.path,
        scenario="staged-once",
        serve_ip="127.0.0.1",
        yes=True,
        dry_run=True,
    )
    token = dry_run.set(False)
    try:
        # reaching return without a RuntimeError is the assertion: the old
        # require_staged_once raised "BUG: no flash output was captured" here
        args.run(
            dev=dev,
            backend=_e2e_nix(tmp_path),
            make_device=lambda _h: dev,
            make_server=_local_server,
        )
    finally:
        dry_run.reset(token)
    cmds = [argv[-1] for argv in exc.runs]
    assert not any(c.startswith("sysupgrade ") for c in cmds)  # flash logged, not run


def _run_with_cleanup_stubs(  # noqa: PLR0913  each cleanup hook controls a distinct failure path
    monkeypatch,
    tmp_path: Path,
    driver,
    restore,
    seen: dict[str, tuple],
    *,
    restore_nix_conf=None,
) -> None:
    """Run the procedure with the preamble and cleanup stages stubbed out
    (recorded in `seen` by name → positional args), a controllable
    restore_server_registry and restore_nix_conf (recorded when not
    overridden), and a fake scenario driver."""
    for fn in (
        "ensure_device_reachable",
        "capture_server_registry",
        "capture_nix_conf",
        "validate_firmware_image",
        "validate_e2e_inputs",
        "build_e2e_artifacts",
        "build_nix_cli",
        "assemble_rig",
        "preflight_rig",
        "cleanup_e2e_marker",
        "sweep_uploaded_images",
        "sweep_shm_upload",
        "cleanup_remote_artifacts",
        "start_compositor",
    ):
        monkeypatch.setattr(faults.catalog, fn, lambda *a, fn=fn, **_k: seen.setdefault(fn, a))
    monkeypatch.setattr(faults.catalog, "restore_server_registry", restore)
    monkeypatch.setattr(
        faults.catalog,
        "restore_nix_conf",
        restore_nix_conf or (lambda *a, **_k: seen.setdefault("restore_nix_conf", a)),
    )
    monkeypatch.setitem(faults._DRIVERS, "unsigned-feed", driver)
    image_a = _e2e_image(tmp_path, name="a.tar", version="va")
    image_b = _e2e_image(tmp_path, name="b.tar", version="vb")
    board = json.dumps({"board_name": "b", "release": {"target": _TARGET}})

    def respond(argv: list[str]):
        return _cp(argv, board if "ubus call system board" in argv[-1] else "")

    dev = Device("127.0.0.1", backend=_Exec(respond))
    E2eSysupgradeFaults(
        device="127.0.0.1",
        image_a=image_a.path,
        image_b=image_b.path,
        scenario="unsigned-feed",
        serve_ip="127.0.0.1",
        yes=True,
    ).run(
        dev=dev,
        backend=_e2e_nix(tmp_path),
        make_device=lambda _h: dev,
        make_server=_local_server,
    )


def test_cleanup_restore_failure_fails_a_successful_run(monkeypatch, tmp_path: Path) -> None:
    """On an otherwise-successful run a failed servers.json restore leaves
    the device registered against the about-to-die rig — best-effort
    swallowing would report success anyway. With no primary failure to
    preserve, the restore failure must fail the run."""

    def driver(ctx) -> None:
        ctx.run.device_mutated = True

    def restore(*_a, **_k) -> None:
        raise Abort("registry restore failed")

    with pytest.raises(Abort, match="registry restore failed"):
        _run_with_cleanup_stubs(monkeypatch, tmp_path, driver, restore, {})


def test_cleanup_restore_failure_never_masks_the_scenario_failure(
    monkeypatch, tmp_path: Path
) -> None:
    """When the scenario itself failed, that failure is the diagnosis — a
    restore failure on top must stay best-effort (logged, not raised)."""

    def driver(ctx) -> None:
        ctx.run.device_mutated = True
        raise Abort("scenario failed")

    def restore(*_a, **_k) -> None:
        raise Abort("registry restore failed")

    with pytest.raises(Abort, match="scenario failed"):
        _run_with_cleanup_stubs(monkeypatch, tmp_path, driver, restore, {})


def test_cleanup_nix_conf_restore_failure_fails_a_successful_run(
    monkeypatch, tmp_path: Path
) -> None:
    """nix.conf carries the rig's trusted signing key — a standing grant
    for a developer machine on a device that outlives the run. Silently
    leaving it behind must fail an otherwise-successful run, exactly like
    a failed servers.json restore."""

    def driver(ctx) -> None:
        ctx.run.device_mutated = True

    def restore_nix_conf(*_a, **_k) -> None:
        raise Abort("nix.conf restore failed")

    with pytest.raises(Abort, match=r"nix\.conf restore failed"):
        _run_with_cleanup_stubs(
            monkeypatch,
            tmp_path,
            driver,
            lambda *_a, **_k: None,
            {},
            restore_nix_conf=restore_nix_conf,
        )


def test_cleanup_stays_strict_inside_a_caller_except_block(monkeypatch, tmp_path: Path) -> None:
    """The strict-vs-best-effort split keys on the run's own failure, not
    on sys.exc_info: invoked from inside a caller's except block (a live
    handled exception), a successful run must still treat a restore
    failure as fatal rather than degrade to best-effort."""

    def driver(ctx) -> None:
        ctx.run.device_mutated = True

    def restore(*_a, **_k) -> None:
        raise Abort("registry restore failed")

    try:
        raise RuntimeError("caller's handled exception")
    except RuntimeError:
        with pytest.raises(Abort, match="registry restore failed"):
            _run_with_cleanup_stubs(monkeypatch, tmp_path, driver, restore, {})


def test_cleanup_runs_on_the_quiesced_pin_after_a_window_failure(
    monkeypatch, tmp_path: Path
) -> None:
    """A failure inside a quiesced window (mDNS dead until a reboot) must
    route every cleanup stage through the surviving numeric pin, not the
    dead name the run was invoked with."""
    pin = Device("10.0.0.99", backend=_Exec(_cp))  # never printed: no board route needed
    seen: dict[str, tuple] = {}
    restores: list[object] = []

    def driver(ctx) -> None:
        ctx.run.device_mutated = True
        ctx.quiesced_pin = pin
        raise Abort("mid-window failure")

    with pytest.raises(Abort, match="mid-window failure"):
        _run_with_cleanup_stubs(
            monkeypatch, tmp_path, driver, lambda dev, _prov: restores.append(dev), seen
        )
    assert seen["cleanup_e2e_marker"][0] is pin
    assert seen["sweep_uploaded_images"][0] is pin
    assert restores == [pin]


def test_cleanup_restarts_the_compositor(monkeypatch, tmp_path: Path) -> None:
    """An abort-expecting scenario stops the compositor and never reboots;
    without a best-effort start in the cleanup the Deck is left with no UI
    and no hint why."""
    seen: dict[str, tuple] = {}

    def driver(ctx) -> None:
        ctx.run.device_mutated = True
        raise Abort("scenario failed")

    with pytest.raises(Abort, match="scenario failed"):
        _run_with_cleanup_stubs(monkeypatch, tmp_path, driver, lambda *_a, **_k: None, seen)
    assert "start_compositor" in seen


def _b1_dry_routes(sha_a: str) -> _Respond:
    """Read-only replies for a blank-data-partition dry-run: as
    _staged_dry_routes, plus the mounted data partition the release stage
    resolves and its blkid identity."""
    board = json.dumps({"board_name": "b", "release": {"target": _TARGET}})
    outputs = {
        "if [ -e /etc/nix-upgrade/servers.json ]": "ABSENT",
        "if [ -e /etc/nix/nix.conf ]": "ABSENT",
        "ubus call system board": board,
        "MemAvailable": "999999999",
        "dd bs=64": "64",
        "sha256sum": sha_a,
        "-d /mnt/data/nix": "yes",  # the store is initialized
        "readlink -f /nix/var/nix/gcroots/profiles/bmc/current": "1-link",
        "cat /etc/bos_version": "va",  # running image A
        "mountinfo": "21 1 179:4 / /mnt/data rw,relatime shared:10 - ext4 /dev/mmcblk0p4 rw",
        "blkid": '/dev/mmcblk0p4: UUID="1111-2222" TYPE="ext4"',
    }

    def respond(argv: list[str]):
        cmd = argv[-1]
        for key, value in outputs.items():
            if key in cmd:
                return _cp(argv, value)
        return _cp(argv)

    return respond


def test_blank_partition_dry_run_skips_the_uuid_proof(tmp_path: Path) -> None:
    """Under --dry-run the mkfs never runs, so the real partition UUID
    cannot have changed; require_fs_uuid_changed must early-return instead
    of aborting with a misleading 'mkfs did not run'. Completing the run is
    the assertion — plus nothing mutating reached the device."""
    image_a = _e2e_image(tmp_path, name="a.tar", version="va")
    image_b = _e2e_image(tmp_path, name="b.tar", version="vb")
    exc = _Exec(_b1_dry_routes(image_a.sha256))
    dev = Device("127.0.0.1", backend=exc)
    args = E2eSysupgradeFaults(
        device="127.0.0.1",
        image_a=image_a.path,
        image_b=image_b.path,
        scenario="blank-data-partition",
        serve_ip="127.0.0.1",
        yes=True,
        dry_run=True,
    )
    token = dry_run.set(False)
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
    assert not any(c.startswith("sysupgrade ") for c in cmds)
    assert not any("mkfs" in c or "dd if=" in c or "umount" in c for c in cmds)


def test_c_group_runs_aborts_then_retry_then_reflash(monkeypatch) -> None:
    calls: list[str] = []
    for sid in (
        "unreachable-rig",
        "malformed-index",
        "wrong-cache-key",
        "full-store",
        "cache-swap-retry",
        "same-version-reflash",
    ):
        monkeypatch.setitem(faults._DRIVERS, sid, lambda _ctx, sid=sid: calls.append(sid))
    faults._group_c(cast("faults._Ctx", types.SimpleNamespace()))
    assert calls == [
        "unreachable-rig",
        "malformed-index",
        "wrong-cache-key",
        "full-store",
        "cache-swap-retry",
        "same-version-reflash",
    ]


def test_matching_image_picks_the_running_version(tmp_path) -> None:
    image_a = _image(tmp_path, name="a.tar", version="va")
    image_b = _image(tmp_path, name="b.tar", version="vb")
    ctx = cast(
        "faults._Ctx",
        types.SimpleNamespace(
            dev=types.SimpleNamespace(version="vb"), image_a=image_a, image_b=image_b
        ),
    )
    assert faults._matching_image(ctx) is image_b


def test_matching_image_aborts_on_foreign_version(tmp_path) -> None:
    image_a = _image(tmp_path, name="a.tar", version="va")
    image_b = _image(tmp_path, name="b.tar", version="vb")
    ctx = cast(
        "faults._Ctx",
        types.SimpleNamespace(
            dev=types.SimpleNamespace(version="vX"), image_a=image_a, image_b=image_b
        ),
    )
    with pytest.raises(Abort, match="neither"):
        faults._matching_image(ctx)


def test_cli_registers_the_procedure() -> None:
    # entrypoint's return type hides __wrapped__ from ty; getattr keeps the
    # gate green while still reading the real command union off the wrapped fn.
    wrapped = getattr(cli.main, "__wrapped__")  # noqa: B009
    assert "E2eSysupgradeFaults" in str(wrapped.__annotations__["command"])


def _note(calls: list[str], name: str):
    return lambda *_a, **_k: calls.append(name)


def test_cache_swap_retry_records_before_tamper_retries_with_riders(monkeypatch) -> None:
    """C1's load-bearing shape: upgrade state recorded BEFORE the cache is
    withheld, the cache restored after the abort, and the retry carries
    the C5 (stale marker), D4 (staged once), and D5 (preservation)
    riders after the upgrade verification. Losing the retry, a rider, or
    the record-before-tamper ordering must fail this test. The narinfo
    cache clear must sit between the restore and the retry flash: the
    withheld-cache attempt poisons the device's persistent negative
    narinfo cache for narinfo-cache-negative-ttl, so an uncleaned retry
    fails without ever re-querying the restored rig cache."""
    calls: list[str] = []
    for fn in (
        "drop_e2e_marker",
        "record_upgrade_state",
        "flash_expect_abort",
        "require_upgrade_state_untouched",
        "clear_nix_narinfo_cache",
        "plant_stale_next_marker",
        "record_generation",
        "record_servers_json",
        "flash_e2e",
        "wait_for_device",
        "verify_upgraded",
        "require_staged_once",
        "require_stale_next_gone",
        "require_preservation_policy",
    ):
        monkeypatch.setattr(faults.catalog, fn, _note(calls, fn))
    monkeypatch.setattr(faults, "_require_c_preconditions", _note(calls, "preconditions"))
    monkeypatch.setattr(faults.catalog, "ensure_bump_absent", _note(calls, "ensure_bump_absent"))
    monkeypatch.setattr(faults, "_prepare_flash", _note(calls, "prepare"))
    monkeypatch.setattr(faults, "_pinned", lambda ctx: ctx.dev)
    monkeypatch.setattr(faults.rig, "swap_cache_away", _note(calls, "swap"))
    monkeypatch.setattr(faults.rig, "restore_cache", _note(calls, "restore"))
    ctx = cast(
        "faults._Ctx",
        types.SimpleNamespace(
            dev=object(),
            run=types.SimpleNamespace(rig=types.SimpleNamespace(cache=object())),
            state=object(),
            image_b=object(),
            yes=True,
            servers_json_preserved=False,
        ),
    )
    faults._scenario_cache_swap_retry(ctx)
    assert calls == [
        "preconditions",
        "ensure_bump_absent",
        "prepare",
        "drop_e2e_marker",
        "record_upgrade_state",
        "swap",
        "flash_expect_abort",
        "require_upgrade_state_untouched",
        "restore",
        "clear_nix_narinfo_cache",
        "plant_stale_next_marker",
        "record_generation",
        "prepare",
        "record_servers_json",
        "flash_e2e",
        "wait_for_device",
        "verify_upgraded",
        "require_staged_once",
        "require_stale_next_gone",
        "require_preservation_policy",
    ]


def test_same_version_reflash_stages_image_b_from_shm(monkeypatch) -> None:
    """C6 must re-flash image B (the running version) from /dev/shm —
    flashing image A here would be a downgrade the platform rejects."""
    flashed: list[tuple[object, object]] = []
    for fn in (
        "require_initialized_store",
        "require_shm_tmpfs",
        "ensure_memory",
        "sweep_uploaded_images",
        "upload_firmware_shm",
        "drop_e2e_marker",
        "record_upgrade_state",
        "wait_for_device",
        "require_rebooted",
        "require_upgrade_state_untouched",
        "require_staged_once",
    ):
        monkeypatch.setattr(faults.catalog, fn, lambda *_a, **_k: None)
    monkeypatch.setattr(faults.catalog, "shm_path", lambda image: f"/dev/shm/{image.version}")
    monkeypatch.setattr(
        faults.catalog,
        "flash_e2e",
        lambda _dev, image, **kw: flashed.append((image, kw.get("remote_path"))),
    )
    monkeypatch.setattr(faults, "_prepare_flash", lambda *_a, **_k: None)
    monkeypatch.setattr(faults, "_pinned", lambda ctx: ctx.dev)
    image_b = types.SimpleNamespace(version="vb", size=1, rootfs_size=1)
    ctx = cast(
        "faults._Ctx",
        types.SimpleNamespace(
            dev=types.SimpleNamespace(version="vb"),
            image_a=types.SimpleNamespace(version="va", size=1, rootfs_size=1),
            image_b=image_b,
            run=object(),
            state=object(),
            yes=True,
        ),
    )
    faults._scenario_same_version_reflash(ctx)
    assert flashed == [(image_b, "/dev/shm/vb")]


def test_prepare_flash_frees_widget_memory_and_sweeps_uploads_before_the_gate(
    monkeypatch, tmp_path: Path
) -> None:
    """The live widget host consumes enough RAM to make the flash headroom
    gate flaky, and stale /tmp uploads are additional tmpfs residents:
    the normal path must stop the compositor and sweep uploads before measuring.
    A quiesced window has already stopped all generation services, but must
    still sweep before measuring."""
    calls: list[str] = []
    for fn in (
        "stop_compositor",
        "push_nix_cli",
        "register_rig",
        "sweep_uploaded_images",
        "sweep_store_ballast",
        "ensure_memory",
        "pin_device_address",
        "upload_firmware",
        "require_uploaded",
        "trust_image_keys",
    ):
        monkeypatch.setattr(faults.catalog, fn, _note(calls, fn))
    monkeypatch.setattr(faults, "_pinned", lambda ctx: ctx.dev)
    image = _image(tmp_path)
    for quiesced_pin in (None, object()):
        calls.clear()
        ctx = cast(
            "faults._Ctx",
            types.SimpleNamespace(
                dev=object(),
                prov=object(),
                run=types.SimpleNamespace(device_mutated=False),
                quiesced_pin=quiesced_pin,
            ),
        )
        faults._prepare_flash(ctx, image)
        if quiesced_pin is None:
            assert calls.index("stop_compositor") < calls.index("ensure_memory")
        else:
            assert "stop_compositor" not in calls
        assert calls.index("sweep_store_ballast") == 0
        assert calls.index("sweep_uploaded_images") < calls.index("ensure_memory")
        assert calls.index("ensure_memory") < calls.index("upload_firmware")
        assert calls.index("sweep_store_ballast") < calls.index("upload_firmware"), (
            "C7 killed outright never reaches its teardown, and the ballast it leaves "
            "fails every later scenario with ENOSPC instead of the message it expects"
        )


def _via_shm_budget_ctx() -> types.SimpleNamespace:
    """A ctx for the via_shm drivers: the device runs image B (vb)."""
    return types.SimpleNamespace(
        dev=types.SimpleNamespace(version="vb"),
        image_a=types.SimpleNamespace(version="va", size=10, rootfs_size=3),
        image_b=types.SimpleNamespace(version="vb", size=10, rootfs_size=3),
        run=object(),
        state=object(),
        yes=True,
    )


def _via_shm_budget_probe(monkeypatch, calls: list[str], budgets: list[int]) -> None:
    """Mock the via_shm driver stages, capturing the prepare memory budget."""
    for fn in (
        "require_initialized_store",
        "require_shm_tmpfs",
        "sweep_uploaded_images",
        "upload_firmware_shm",
        "drop_e2e_marker",
        "record_upgrade_state",
        "flash_e2e",
        "wait_for_device",
        "require_rebooted",
        "require_upgrade_state_untouched",
        "require_staged_once",
        "shm_path",
    ):
        monkeypatch.setattr(faults.catalog, fn, _note(calls, fn))

    def prepare(*_args, memory_need: int | None = None, **_kwargs) -> None:
        calls.append("prepare")
        assert memory_need is not None
        budgets.append(memory_need)

    monkeypatch.setattr(faults, "_prepare_flash", prepare)
    monkeypatch.setattr(faults, "_pinned", lambda ctx: ctx.dev)


@pytest.mark.parametrize(
    "driver", [faults._scenario_same_version_reflash, faults._scenario_shm_local_file]
)
def test_via_shm_budgets_two_copies_and_frees_the_tmp_upload(monkeypatch, driver) -> None:
    """C6/D1 on the 256 MiB Deck: the /tmp upload only feeds trust_image_keys,
    so it is deleted right after _prepare_flash and BEFORE the /dev/shm
    staging — only two image-sized tmpfs residents coexist during the flash
    (the /dev/shm copy and sysupgrade's /tmp/sysupgrade.img), and the memory
    gate must ask for exactly that (a 3x figure exceeds the device's ~85 MiB
    fresh-boot headroom and can never run)."""
    calls: list[str] = []
    budgets: list[int] = []
    _via_shm_budget_probe(monkeypatch, calls, budgets)
    driver(cast("faults._Ctx", _via_shm_budget_ctx()))
    assert budgets == [2 * 10 + 3 + faults._FLASH_HEADROOM]
    assert (
        calls.index("prepare")
        < calls.index("sweep_uploaded_images")
        < calls.index("upload_firmware_shm")
    )


def test_upgrade_abort_restores_rig_even_when_the_abort_check_fails(monkeypatch) -> None:
    """The untouched-state contract: preconditions and the state record
    run before the tamper, and _restore_rig runs even when the flash
    attempt itself raises."""
    calls: list[str] = []
    for fn in ("drop_e2e_marker", "record_upgrade_state"):
        monkeypatch.setattr(faults.catalog, fn, _note(calls, fn))
    monkeypatch.setattr(faults, "_require_c_preconditions", _note(calls, "preconditions"))
    monkeypatch.setattr(faults, "_prepare_flash", _note(calls, "prepare"))
    monkeypatch.setattr(faults, "_pinned", lambda ctx: ctx.dev)
    monkeypatch.setattr(faults, "_restore_rig", _note(calls, "restore-rig"))

    def exploding_flash(*_a, **_k) -> None:
        raise Abort("session death")

    monkeypatch.setattr(faults.catalog, "flash_expect_abort", exploding_flash)
    ctx = cast(
        "faults._Ctx",
        types.SimpleNamespace(dev=object(), state=object(), image_b=object(), yes=True),
    )
    with pytest.raises(Abort, match="session death"):
        faults._attempt_upgrade_abort(ctx, tamper=_note(calls, "tamper"), expect="x")
    assert calls == [
        "preconditions",
        "prepare",
        "drop_e2e_marker",
        "record_upgrade_state",
        "tamper",
        "restore-rig",
    ]


def _full_store_stubs(monkeypatch, calls: list[str]) -> "faults._Ctx":
    monkeypatch.setattr(faults, "_require_c_preconditions", _note(calls, "preconditions"))
    monkeypatch.setattr(faults, "_prepare_flash", _note(calls, "prepare"))
    monkeypatch.setattr(faults, "_pinned", lambda ctx: ctx.dev)
    for fn in (
        "ensure_bump_absent",
        "drop_e2e_marker",
        "record_upgrade_state",
        "require_upgrade_state_untouched",
        "fill_store_filesystem",
        "sweep_store_ballast",
    ):
        monkeypatch.setattr(faults.catalog, fn, _note(calls, fn))
    return cast(
        "faults._Ctx",
        types.SimpleNamespace(
            dev=object(), run=object(), state=object(), image_b=object(), yes=True
        ),
    )


def test_full_store_releases_the_ballast_when_the_flash_check_fails(monkeypatch) -> None:
    """A ballast left behind wedges the store for every later scenario,
    so the release must run even when the attempt raises —
    and only after the state record,
    or the recorded 'before' would already be the full-disk state."""
    calls: list[str] = []
    ctx = _full_store_stubs(monkeypatch, calls)

    def exploding_flash(*_a, **_k) -> None:
        raise Abort("session death")

    monkeypatch.setattr(faults.catalog, "flash_expect_abort", exploding_flash)
    with pytest.raises(Abort, match="session death"):
        faults._scenario_full_store(ctx)
    assert calls == [
        "preconditions",
        "ensure_bump_absent",
        "prepare",
        "drop_e2e_marker",
        "record_upgrade_state",
        "fill_store_filesystem",
        "sweep_store_ballast",
    ]


def test_full_store_release_failure_fails_a_successful_attempt(monkeypatch) -> None:
    """With no primary failure to preserve, a ballast that cannot be removed
    must fail the run: swallowing it hands every later scenario a full store."""
    calls: list[str] = []
    ctx = _full_store_stubs(monkeypatch, calls)
    monkeypatch.setattr(faults.catalog, "flash_expect_abort", lambda *_a, **_k: None)

    def exploding_release(*_a, **_k) -> None:
        raise OSError("rm failed")

    monkeypatch.setattr(faults.catalog, "sweep_store_ballast", exploding_release)
    with pytest.raises(OSError, match="rm failed"):
        faults._scenario_full_store(ctx)


def _upgrade_abort_stubs(monkeypatch) -> "faults._Ctx":
    monkeypatch.setattr(faults, "_require_c_preconditions", lambda _ctx: None)
    monkeypatch.setattr(faults, "_prepare_flash", lambda _ctx, _image: None)
    monkeypatch.setattr(faults, "_pinned", lambda ctx: ctx.dev)
    for fn in ("drop_e2e_marker", "record_upgrade_state", "require_upgrade_state_untouched"):
        monkeypatch.setattr(faults.catalog, fn, lambda *_a, **_k: None)
    return cast(
        "faults._Ctx",
        types.SimpleNamespace(dev=object(), state=object(), image_b=object(), yes=True),
    )


def test_upgrade_abort_restore_failure_cannot_mask_the_flash_failure(monkeypatch) -> None:
    """When the attempt itself raised, that Abort is the diagnosis — a rig
    restore failing on top of it must be logged, not raised in its place."""
    ctx = _upgrade_abort_stubs(monkeypatch)

    def exploding_flash(*_a, **_k) -> None:
        raise Abort("session death")

    def exploding_restore(_ctx) -> None:
        raise OSError("rename failed")

    monkeypatch.setattr(faults.catalog, "flash_expect_abort", exploding_flash)
    monkeypatch.setattr(faults, "_restore_rig", exploding_restore)
    with pytest.raises(Abort, match="session death"):
        faults._attempt_upgrade_abort(ctx, tamper=lambda: None, expect="x")


def test_upgrade_abort_restore_failure_fails_a_successful_attempt(monkeypatch) -> None:
    """With no primary failure to preserve, a broken rig restore must fail
    the scenario — swallowing it would hand every later scenario a
    tampered rig."""
    ctx = _upgrade_abort_stubs(monkeypatch)
    monkeypatch.setattr(faults.catalog, "flash_expect_abort", lambda *_a, **_k: None)

    def exploding_restore(_ctx) -> None:
        raise OSError("rename failed")

    monkeypatch.setattr(faults, "_restore_rig", exploding_restore)
    with pytest.raises(OSError, match="rename failed"):
        faults._attempt_upgrade_abort(ctx, tamper=lambda: None, expect="x")

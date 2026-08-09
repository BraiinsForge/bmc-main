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

"""Fault-injection e2e sysupgrade suite against a real Deck: the happy
path's init/upgrade flows re-run under deliberate rig faults (refusing or
stalling server, stripped or wrong signatures, corrupt tarball/index,
swapped-away cache) plus device-side surgeries (partition swaps, staged
/dev/shm uploads), asserting COMMAND aborts cleanly and leaves the store
recoverable.

This module holds the pinned all-suite order, the scenario dispatch
registry, the shared preamble/cleanup, the rig-restore and dry-run tamper
seams, and the full set of A/B/C/D scenario drivers.
"""

import subprocess
import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from bmc_tui import catalog, console, nix, rig
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.nix import Nix
from bmc_tui.server import default_serve_ip
from bmc_tui.stage import Abort, dry_run, entrypoint, require

# sysupgrade stages the tar in /tmp (tmpfs) and pivots to a ramdisk; same
# headroom rationale as procedures/e2e_sysupgrade.py. Private there, so
# duplicated rather than imported.
_FLASH_HEADROOM = 20 * 1024 * 1024

# Scenario ids are descriptive slugs; the design doc's matrix ids (A1…D5)
# appear as trailing comments and stay canonical in docs/tickets.
Scenario = Literal[
    "all",
    "a",
    "b",
    "c",
    "d",
    "wrong-key-signature",  # A1
    "unsigned-feed",  # A2
    "untrusted-key-name",  # A3
    "corrupt-tarball",  # A4
    "download-stall",  # A5
    "blank-data-partition",  # B1
    "corrupt-fs-metadata",  # B2
    "store-remnants",  # B3
    "missing-store-db",  # B4
    "unmounted-store",  # B5
    "cache-swap-retry",  # C1
    "unreachable-rig",  # C2
    "malformed-index",  # C3
    "wrong-cache-key",  # C4
    "stale-next-marker",  # C5
    "same-version-reflash",  # C6
    "full-store",  # C7
    "shm-local-file",  # D1
    "staged-once",  # D4
    "servers-json",  # D5
]

# The pinned per-group orders the group runners iterate: fault scenarios
# grouped by init/upgrade family, with cheaper faults first so a common
# regression surfaces early. cache-swap-retry (C1) fuses stale-next-marker
# (C5) + servers-json (D5) onto its retry flash; same-version-reflash (C6)
# fuses shm-local-file (D1); the fused ids therefore have drivers but do
# not appear here.
_GROUP_A_ORDER = (
    "unsigned-feed",
    "untrusted-key-name",
    "wrong-key-signature",
    "corrupt-tarball",
    "download-stall",
)
_GROUP_B_ORDER = (
    "store-remnants",
    "missing-store-db",
    "unmounted-store",
    "blank-data-partition",
    "corrupt-fs-metadata",
)
_GROUP_C_ORDER = (
    "unreachable-rig",
    "malformed-index",
    "wrong-cache-key",
    "full-store",  # leaves the device on A; cache-swap-retry proves B still lands
    "cache-swap-retry",  # fuses stale-next-marker + servers-json on retry
    "same-version-reflash",  # fuses shm-local-file (staged via /dev/shm)
)
# `good-init` is the clean init flash that closes group A — a flash step,
# not a scenario driver.
SUITE_ORDER = (*_GROUP_A_ORDER, "good-init", *_GROUP_B_ORDER, *_GROUP_C_ORDER)


def _best_effort(action: Callable[[], object]) -> None:
    """Cleanup must not mask the failure that triggered it."""
    try:
        action()
    except Exception as e:
        console.kv("cleanup failed", str(e))


def _restore_step(*, failed: bool, action: Callable[[], object]) -> None:
    """A restore that must fail the run when the body succeeded — silently
    leaving the rig broken would poison every later scenario — but must
    not replace a primary failure being unwound."""
    if failed:
        _best_effort(action)
    else:
        action()


@dataclass
class _Ctx:
    dev: Device
    nix: Nix
    run: catalog.E2eRun
    prov: catalog.Provisioning  # holds the device-arch CLI; push_nix_cli takes it
    state: catalog.FaultsState
    server: rig.RigServer
    image_a: Image
    image_b: Image
    make_device: Callable[[str], Device]
    yes: bool
    servers_json_preserved: bool
    # Set for a quiesced window (group A's up-front cleardown, a B-recovery
    # re-flash): the mDNS name is dead, so every stage runs on this pre-pinned
    # numeric handle and nothing re-resolves the name until the next reboot.
    quiesced_pin: Device | None = None


def _pinned(ctx: _Ctx) -> Device:
    if ctx.run.pinned_host is None or ctx.run.pinned_host == ctx.dev.host:
        return ctx.dev
    return ctx.make_device(ctx.run.pinned_host)


def _restore_rig(ctx: _Ctx) -> None:
    """Undo every reversible tamper: good serve tree, fault mode off,
    cache back in place. Registration restore happens by re-running
    register_rig before the next flash."""
    r = ctx.run.rig
    a, b = ctx.run.variant_a, ctx.run.variant_b
    if r is None or a is None or b is None:
        return
    ctx.server.set_fault(rig.FaultMode.NONE)
    rig.restore_cache(r.cache)
    rig.write_serve_root(r.serve_root, [a, b], r.base_url)


def _host_tamper(action: Callable[[], None]) -> None:
    """Host-side rig tampering (feed rewrites, cache swaps, tarball/index
    corruption, wrong-key generation) honors --dry-run like device
    stages do: the rig writers are plain functions, not dry-run-aware
    stages, so every driver routes them through here — logged, never
    executed."""
    if dry_run.get():
        console.kv("tamper", "skipped (dry-run)")
        return
    action()


def _prepare_flash(ctx: _Ctx, image: Image, *, memory_need: int | None = None) -> Device:
    """Everything a flash attempt needs, idempotent per attempt: CLI +
    registration, widget teardown, RAM headroom, pinned address, verified
    upload, trusted keys. Returns the handle to flash through.

    Normally (`quiesced_pin` unset, mDNS alive) this pins the name to a
    numeric address and prepares via the name, mirroring the happy path.
    Inside a quiesced window (`quiesced_pin` set — group A's up-front
    cleardown stopped avahi, or a B-recovery re-flash) the name is dead, so
    every stage runs on the pre-pinned numeric handle and nothing re-resolves
    the name."""
    ctx.run.device_mutated = True
    # Sweep stale /tmp uploads from earlier scenarios BEFORE the memory gate:
    # they are tmpfs residents (RAM), so without the sweep the gate demands a
    # fresh image-size of headroom on top of an image it is about to overwrite
    # — double-counting one image size for every scenario after the first
    # (observed as a spurious abort on the 256 MiB Deck). Re-uploading per
    # scenario is the accepted cost.
    need = (
        memory_need if memory_need is not None else image.size + image.rootfs_size + _FLASH_HEADROOM
    )
    if ctx.quiesced_pin is not None:
        pinned = ctx.quiesced_pin
        catalog.sweep_store_ballast(pinned)
        catalog.push_nix_cli(pinned, ctx.prov)
        catalog.register_rig(pinned, ctx.run)
        catalog.sweep_uploaded_images(pinned, ctx.run)
        catalog.ensure_memory(pinned, need)
        catalog.upload_firmware(pinned, image)
    else:
        catalog.sweep_store_ballast(ctx.dev)
        # Production stops widgets before its tmpfs firmware download. The SSH
        # path has no widget-lifecycle handle, so stop the owning service; its
        # graceful shutdown releases bmc-wasm-host while keeping /nix mounted.
        catalog.stop_compositor(ctx.dev)
        catalog.push_nix_cli(ctx.dev, ctx.prov)
        catalog.register_rig(ctx.dev, ctx.run)
        catalog.sweep_uploaded_images(ctx.dev, ctx.run)
        catalog.ensure_memory(ctx.dev, need)
        catalog.pin_device_address(ctx.dev, ctx.run)
        pinned = _pinned(ctx)
        catalog.upload_firmware(ctx.dev, image)
    catalog.require_uploaded(pinned, image)
    catalog.trust_image_keys(pinned, image)
    return pinned


def _tarball_a(ctx: _Ctx) -> str:
    """Image A's SERVED tarball filename — for rig-side tampering only.
    The on-device download path is fixed (init-tarball.tar.gz) and never
    carries this name; the artifact stages take no name for that reason."""
    a = ctx.run.variant_a
    if a is None:
        msg = "BUG: variants were not built before the A-group"
        raise RuntimeError(msg)
    return a.tarball.name


def _attempt_init_abort(
    ctx: _Ctx,
    *,
    tamper: Callable[[], None],
    expect: str | tuple[str, ...],
    artifact_deleted: bool = True,
) -> None:
    """One tampered init attempt: precondition, tamper, expect-abort flash
    of image A, then the observables: store absent, and (by default) the
    fixed-path download artifact absent. The artifact assertion is
    device behavior, not cleanup: A2 aborts before any bytes are fetched
    (missing feed signature), and A1/A3/A4's tarball is deleted by
    bmc-nix itself on signature rejection — including A3, which downloads
    the full tarball before the key-name mismatch is caught (store.rs).
    A5 passes artifact_deleted=False — the stall/download-error paths
    return WITHOUT deleting the partial file, so only the finally-sweep
    (which always runs, pass or fail) cleans it there."""
    catalog.require_store_absent(ctx.quiesced_pin or ctx.dev)
    pinned = _prepare_flash(ctx, ctx.image_a)
    failed = False
    try:
        _host_tamper(tamper)
        catalog.sweep_download_artifact(pinned)
        catalog.flash_expect_abort(
            pinned, ctx.image_a, expect=expect, state=ctx.state, assume_yes=ctx.yes
        )
        catalog.require_store_absent(pinned)
        if artifact_deleted:
            catalog.require_download_artifact_absent(pinned)
    except BaseException:
        failed = True
        raise
    finally:
        # a cleanup error must not replace the real Abort the flash raised
        _best_effort(lambda: catalog.sweep_download_artifact(pinned))
        _restore_step(failed=failed, action=lambda: _restore_rig(ctx))


def _serve_root(ctx: _Ctx) -> Path:
    r = ctx.run.rig
    if r is None:
        msg = "BUG: the rig was not assembled before tampering"
        raise RuntimeError(msg)
    return r.serve_root


def _wrong_key_signature(ctx: _Ctx) -> str:
    """A signature over image A's tarball by a fresh key under the trusted
    key NAME — verification must fail on the bytes, not the name (A1)."""
    r = ctx.run.rig
    a = ctx.run.variant_a
    if r is None or a is None:
        msg = "BUG: the rig was not assembled before tampering"
        raise RuntimeError(msg)
    wrong_secret = r.secret.with_name("wrong-key.secret")
    ctx.nix.generate_cache_key(rig.CACHE_KEY_NAME, wrong_secret)
    signed = rig.sign_variant(r.host_cli, wrong_secret, a)
    if signed.signature is None:
        msg = "BUG: sign_variant returned an unsigned variant"
        raise RuntimeError(msg)
    return signed.signature


def _scenario_wrong_key_signature(ctx: _Ctx) -> None:
    # the wrong-key generation happens inside the tamper lambda so the
    # dry-run guard in _host_tamper covers it too
    _attempt_init_abort(
        ctx,
        tamper=lambda: rig.set_feed_signatures(_serve_root(ctx), _wrong_key_signature(ctx)),
        expect="init tarball signature verification failed",
    )


def _scenario_unsigned_feed(ctx: _Ctx) -> None:
    _attempt_init_abort(
        ctx,
        tamper=lambda: rig.strip_feed_signatures(_serve_root(ctx)),
        expect="has no signature",
    )


def _scenario_untrusted_key_name(ctx: _Ctx) -> None:
    a = ctx.run.variant_a
    if a is None or a.signature is None:
        msg = "BUG: variant A is unsigned"
        raise RuntimeError(msg)
    renamed = "e2e-wrong-name:" + a.signature.split(":", 1)[1]
    _attempt_init_abort(
        ctx,
        tamper=lambda: rig.set_feed_signatures(_serve_root(ctx), renamed),
        expect="does not match trusted key name",
    )


def _scenario_corrupt_tarball(ctx: _Ctx) -> None:
    _attempt_init_abort(
        ctx,
        tamper=lambda: rig.corrupt_tarball(_serve_root(ctx), _tarball_a(ctx)),
        expect="init tarball signature verification failed",
    )


def _scenario_download_stall(ctx: _Ctx) -> None:
    # artifact_deleted=False: a stalled download returns without deleting
    # the partial file (store.rs) — the attempt's finally-sweep cleans it
    _attempt_init_abort(
        ctx,
        tamper=lambda: ctx.server.set_fault(rig.FaultMode.STALL),
        expect=("download stalled", "tarball download failed"),
        artifact_deleted=False,
    )


def _flash_good_init(ctx: _Ctx) -> None:
    """Good-rig flash of image A + full init verification — the A-group
    finale (recovery proof) and the B-group's recovery flash."""
    pinned = _prepare_flash(ctx, ctx.image_a)
    catalog.flash_e2e(pinned, ctx.image_a, assume_yes=ctx.yes, state=ctx.state)
    # the flash reboots: the numeric pin may not survive a new DHCP lease, so
    # read-backs return to the mDNS name once the device answers again
    ctx.quiesced_pin = None
    catalog.wait_for_device(ctx.dev)
    catalog.verify_initialized(ctx.dev, ctx.run)
    catalog.require_staged_once(ctx.state)


def _group_a(ctx: _Ctx) -> None:
    # Pin BEFORE the cleardown and run it (and the whole group) on the numeric
    # handle: the cleardown's quiesce stops avahi, so the mDNS name goes dead
    # for the rest of the group — no reboot restores it until the good-init
    # finale. device_mutated is set first so a mid-cleardown failure still
    # runs the outer cleanup. Mirrors the happy path (e2e_sysupgrade.py).
    ctx.run.device_mutated = True
    catalog.pin_device_address(ctx.dev, ctx.run)
    ctx.quiesced_pin = _pinned(ctx)
    # Deliberately no finally-clear: a failure inside the window must leave
    # the pin set — the mDNS name stays dead until a reboot, and the outer
    # cleanup (servers.json restore included) needs the numeric handle to
    # reach the device at all. _flash_good_init clears it on the way out.
    catalog.clear_nix_store(ctx.quiesced_pin, assume_yes=ctx.yes)
    for sid in _GROUP_A_ORDER:
        _DRIVERS[sid](ctx)
    _flash_good_init(ctx)  # clears quiesced_pin before its post-reboot wait


def _require_b_preconditions(ctx: _Ctx) -> None:
    """Read-only: running image A's firmware with an initialized store —
    asserted before any mutation so a mis-sequenced invocation aborts
    with the device untouched."""
    catalog.require_lineage(ctx.dev, ctx.run)
    catalog.require_initialized_store(ctx.dev)


def _scenario_blank_data_partition(ctx: _Ctx) -> None:
    _require_b_preconditions(ctx)
    _prepare_flash(ctx, ctx.image_a)
    pinned = _pinned(ctx)  # quiesce stops services: mDNS may die — numeric address only
    catalog.quiesce_nix(pinned)
    catalog.release_data_partition(pinned, ctx.state)
    catalog.corrupt_partition_blank(pinned, ctx.state)
    _flash_and_verify_init(ctx)
    catalog.require_fs_uuid_changed(ctx.dev, ctx.state)  # post-reboot: mDNS is back


def _scenario_corrupt_fs_metadata(ctx: _Ctx) -> None:
    # OpenWRT's e2fsprogs ships no debugfs, so instead of requiring it on the
    # device the harness cross-builds a static one (lazy — only here) and
    # pushes it; the corruption stage runs that pushed binary.
    _require_b_preconditions(ctx)
    catalog.build_debugfs(ctx.nix, ctx.prov)
    _prepare_flash(ctx, ctx.image_a)
    pinned = _pinned(ctx)
    catalog.push_debugfs(pinned, ctx.prov)
    catalog.quiesce_nix(pinned)
    catalog.release_data_partition(pinned, ctx.state)
    catalog.corrupt_partition_metadata(pinned, ctx.state)
    _flash_and_verify_init(ctx)
    catalog.require_fs_uuid_unchanged(ctx.dev, ctx.state)


def _scenario_store_remnants(ctx: _Ctx) -> None:
    _require_b_preconditions(ctx)
    _prepare_flash(ctx, ctx.image_a)
    pinned = _pinned(ctx)
    catalog.quiesce_nix(pinned)
    catalog.plant_store_remnants(pinned)
    _flash_and_verify_init(ctx)
    catalog.require_remnants_gone(ctx.dev)


def _scenario_missing_store_db(ctx: _Ctx) -> None:
    _require_b_preconditions(ctx)
    _prepare_flash(ctx, ctx.image_a)
    pinned = _pinned(ctx)
    catalog.quiesce_nix(pinned)
    catalog.plant_store_witness(pinned)
    catalog.delete_store_db(pinned)
    _flash_and_verify_init(ctx)
    catalog.require_witness_gone(ctx.dev)


def _scenario_unmounted_store(ctx: _Ctx) -> None:
    _require_b_preconditions(ctx)
    _prepare_flash(ctx, ctx.image_a)
    pinned = _pinned(ctx)
    catalog.quiesce_nix(pinned)
    catalog.plant_store_witness(pinned)  # store intact, /nix just unmounted
    _flash_and_verify_init(ctx)
    catalog.require_witness_gone(ctx.dev)  # pins the status-3 wipe behavior


def _flash_and_verify_init(ctx: _Ctx) -> None:
    """Flash image A and verify a fresh init — the B-scenario tail. The
    upload/keys/registration ran in _prepare_flash before the surgery,
    so this flashes from the already-verified /tmp upload via the pinned
    address (mDNS is down after the quiesce)."""
    catalog.flash_e2e(_pinned(ctx), ctx.image_a, assume_yes=ctx.yes, state=ctx.state)
    catalog.wait_for_device(ctx.dev)
    catalog.verify_initialized(ctx.dev, ctx.run)
    catalog.require_staged_once(ctx.state)


def _with_b_recovery(ctx: _Ctx, scenario: Callable[[_Ctx], None]) -> None:
    """The B-group recovery contract: on failure the device is left over
    ssh with services stopped and a damaged/absent store — attempt one
    good-rig re-flash of image A, then re-raise the original failure.
    Covers Abort (assertion failures) AND CalledProcessError (dd,
    debugfs, umount, or a flash command dying over ssh) — the surgery
    steps raise the latter, and they must trigger the same one-shot
    recovery."""
    try:
        scenario(ctx)
    except (Abort, subprocess.CalledProcessError) as failure:
        hint = failure.hint if isinstance(failure, Abort) else str(failure)
        console.warn(f"B-scenario failed ({hint}) — attempting the one-shot recovery flash")
        # Recovery fires with the device quiesced (mDNS possibly dead), but the
        # failed scenario already pinned a numeric handle: reuse it so the
        # recovery's prepare stages don't re-resolve a dead name.
        ctx.quiesced_pin = _pinned(ctx)
        # No finally-clear: a successful recovery clears the pin inside
        # _flash_good_init; a failed one must leave it set so the outer
        # cleanup still reaches the (quiesced, mDNS-dead) device.
        try:
            _flash_good_init(ctx)
        except (Abort, subprocess.CalledProcessError) as recovery:
            raise Abort(
                f"scenario failed AND the recovery flash failed ({recovery}); "
                f"original failure: {hint}. Manual fallback: re-flash image A "
                "with the good rig, or run `deck init --wipe` semantics via "
                "bmc-nix-cli init --wipe on the device"
            ) from failure
        raise


def _group_b(ctx: _Ctx) -> None:
    for sid in _GROUP_B_ORDER:
        _with_b_recovery(ctx, _DRIVERS[sid])


def _require_c_preconditions(ctx: _Ctx) -> None:
    catalog.require_lineage(ctx.dev, ctx.run)  # running image A
    catalog.require_initialized_store(ctx.dev)


def _attempt_upgrade_abort(
    ctx: _Ctx, *, tamper: Callable[[], None], expect: str | tuple[str, ...]
) -> None:
    """One tampered upgrade attempt of image B: record → tamper →
    expect-abort → untouched-state contract → restore. Post-pin stages
    use the pinned device (the plan-wide rule; no reboot happens, so
    the pin stays valid through the whole window)."""
    _require_c_preconditions(ctx)
    _prepare_flash(ctx, ctx.image_b)
    pinned = _pinned(ctx)
    catalog.drop_e2e_marker(pinned)
    catalog.record_upgrade_state(pinned, ctx.state)
    failed = False
    try:
        _host_tamper(tamper)
        catalog.flash_expect_abort(
            pinned, ctx.image_b, expect=expect, state=ctx.state, assume_yes=ctx.yes
        )
        catalog.require_upgrade_state_untouched(pinned, ctx.state)
    except BaseException:
        failed = True
        raise
    finally:
        _restore_step(failed=failed, action=lambda: _restore_rig(ctx))


def _scenario_unreachable_rig(ctx: _Ctx) -> None:
    _attempt_upgrade_abort(
        ctx,
        tamper=lambda: ctx.server.set_fault(rig.FaultMode.REFUSE),
        expect="package feed fetch for firmware",
    )


def _scenario_malformed_index(ctx: _Ctx) -> None:
    b = ctx.run.variant_b
    if b is None:
        msg = "BUG: variants were not built before the C-group"
        raise RuntimeError(msg)
    _attempt_upgrade_abort(
        ctx,
        tamper=lambda: rig.corrupt_index(_serve_root(ctx), b.bos_version),
        expect="invalid index JSON",
    )


def _scenario_wrong_cache_key(ctx: _Ctx) -> None:
    _require_c_preconditions(ctx)
    catalog.ensure_bump_absent(ctx.dev, ctx.run)
    _prepare_flash(ctx, ctx.image_b)
    pinned = _pinned(ctx)
    catalog.drop_e2e_marker(pinned)
    catalog.record_upgrade_state(pinned, ctx.state)
    r = ctx.run.rig
    if r is None:
        msg = "BUG: the rig was not assembled before C4"
        raise RuntimeError(msg)
    if dry_run.get():
        # wrong-key generation is host-side tamper setup: logged, not done
        wrong_public = f"{rig.CACHE_KEY_NAME}:dry-run"
    else:
        wrong_secret = r.secret.with_name("wrong-key.secret")
        wrong_public = ctx.nix.generate_cache_key(rig.CACHE_KEY_NAME, wrong_secret)
    failed = False
    try:
        catalog.register_rig_tampered(pinned, ctx.run, wrong_public_key=wrong_public)
        catalog.flash_expect_abort(
            pinned,
            ctx.image_b,
            expect=("no substituter provides", "--realise failed"),
            state=ctx.state,
            assume_yes=ctx.yes,
        )
        catalog.require_upgrade_state_untouched(pinned, ctx.state)
    except BaseException:
        failed = True
        raise
    finally:
        # good-key restore; a re-registration error must not replace the real Abort
        _restore_step(failed=failed, action=lambda: catalog.register_rig(pinned, ctx.run))


def _scenario_cache_swap_retry(ctx: _Ctx) -> None:
    """C1 + the riders: cache withheld → abort; swap back → retry flashes B
    with a stale next marker planted (C5); D5's preservation checks and
    D4's staged-once ride the retry's first boot into image B."""
    _require_c_preconditions(ctx)
    catalog.ensure_bump_absent(ctx.dev, ctx.run)
    r = ctx.run.rig
    if r is None:
        msg = "BUG: the rig was not assembled before C1"
        raise RuntimeError(msg)
    _prepare_flash(ctx, ctx.image_b)
    pinned = _pinned(ctx)
    catalog.drop_e2e_marker(pinned)
    catalog.record_upgrade_state(pinned, ctx.state)
    failed = False
    try:
        _host_tamper(lambda: rig.swap_cache_away(r.cache))
        catalog.flash_expect_abort(
            pinned,
            ctx.image_b,
            expect=("no substituter provides", "--realise failed"),
            state=ctx.state,
            assume_yes=ctx.yes,
        )
        catalog.require_upgrade_state_untouched(pinned, ctx.state)
    except BaseException:
        failed = True
        raise
    finally:
        # idempotent: a no-op when the swap was skipped
        _restore_step(failed=failed, action=lambda: rig.restore_cache(r.cache))
    # The withheld-cache attempt left a NEGATIVE narinfo entry in the device's
    # persistent on-disk lookup cache, which would poison the retry for up to
    # narinfo-cache-negative-ttl (3600 s) — the restored rig cache is never
    # re-queried. Clearing it models the TTL expiring / a later retry.
    catalog.clear_nix_narinfo_cache(pinned)
    # retry with the riders
    catalog.plant_stale_next_marker(pinned)  # C5
    catalog.record_generation(pinned, ctx.run)
    _prepare_flash(ctx, ctx.image_b)
    catalog.record_servers_json(_pinned(ctx), ctx.state)  # D5
    catalog.flash_e2e(_pinned(ctx), ctx.image_b, assume_yes=ctx.yes, state=ctx.state)
    catalog.wait_for_device(ctx.dev)
    catalog.verify_upgraded(ctx.dev, ctx.run)
    catalog.require_staged_once(ctx.state)  # D4
    catalog.require_stale_next_gone(ctx.dev)  # C5
    catalog.require_preservation_policy(  # D5
        ctx.dev, ctx.state, servers_json_preserved=ctx.servers_json_preserved
    )


def _scenario_full_store(ctx: _Ctx) -> None:
    """C7: the store's filesystem has no room for the incoming generation.

    The staging preflight must refuse the plan before fetching anything,
    so sysupgrade aborts before flashing
    and the running system is left exactly as it was."""
    _require_c_preconditions(ctx)
    catalog.ensure_bump_absent(ctx.dev, ctx.run)
    _prepare_flash(ctx, ctx.image_b)
    pinned = _pinned(ctx)
    catalog.drop_e2e_marker(pinned)
    catalog.record_upgrade_state(pinned, ctx.state)
    failed = False
    try:
        catalog.fill_store_filesystem(pinned)
        catalog.flash_expect_abort(
            pinned,
            ctx.image_b,
            expect="not enough space in the store",
            state=ctx.state,
            assume_yes=ctx.yes,
        )
        catalog.require_upgrade_state_untouched(pinned, ctx.state)
    except BaseException:
        failed = True
        raise
    finally:
        _restore_step(failed=failed, action=lambda: catalog.sweep_store_ballast(pinned))


def _scenario_same_version_reflash(ctx: _Ctx) -> None:
    """C6 + D1: same-version re-flash of image B staged via /dev/shm —
    empty plan, no next marker, current unchanged, marker survived."""
    _same_version_reflash(ctx, via_shm=True, expect_image=ctx.image_b)


def _matching_image(ctx: _Ctx) -> Image:
    version = ctx.dev.version
    for image in (ctx.image_a, ctx.image_b):
        if image.version == version:
            return image
    raise Abort(
        f"device runs {console.lit(version)} which matches neither supplied image "
        f"({console.lit(ctx.image_a.version)}, {console.lit(ctx.image_b.version)})"
    )


def _same_version_reflash(ctx: _Ctx, *, via_shm: bool, expect_image: Image | None = None) -> None:
    """Same-version re-flash of the image matching the running firmware
    (never 'the other image' — that could be a downgrade, which the
    platform rejects). `expect_image` replaces the lookup with a hard
    assert that the device runs exactly that image, instead of
    re-flashing whatever it happens to run."""
    catalog.require_initialized_store(ctx.dev)
    if expect_image is None:
        image = _matching_image(ctx)
    else:
        require(
            ctx.dev.version == expect_image.version,
            f"device runs {ctx.dev.version}, not the expected {expect_image.version}",
        )
        image = expect_image
    memory_need: int | None = None
    if via_shm:
        catalog.require_shm_tmpfs(ctx.dev)
        # 2x image: the /tmp upload from _prepare_flash is deleted right after
        # trust_image_keys, so only the /dev/shm staging copy and sysupgrade's
        # own /tmp/sysupgrade.img coexist during the flash — all tmpfs (RAM).
        memory_need = 2 * image.size + image.rootfs_size + _FLASH_HEADROOM
    _prepare_flash(ctx, image, memory_need=memory_need)
    pinned = _pinned(ctx)
    if via_shm:
        catalog.sweep_uploaded_images(pinned, ctx.run)  # the /tmp upload only fed trust_image_keys
        catalog.upload_firmware_shm(pinned, image)
    catalog.drop_e2e_marker(pinned)
    catalog.record_upgrade_state(pinned, ctx.state)
    catalog.flash_e2e(
        pinned,
        image,
        assume_yes=ctx.yes,
        remote_path=catalog.shm_path(image) if via_shm else None,
        state=ctx.state,
    )
    catalog.wait_for_device(ctx.dev)
    catalog.require_rebooted(ctx.dev, ctx.state)  # a no-op sysupgrade would pass every check below
    require(ctx.dev.version == image.version, "version changed on a same-version re-flash")
    catalog.require_upgrade_state_untouched(ctx.dev, ctx.state)
    catalog.require_staged_once(ctx.state)


def _scenario_shm_local_file(ctx: _Ctx) -> None:
    _same_version_reflash(ctx, via_shm=True)


def _scenario_staged_once(ctx: _Ctx) -> None:
    _same_version_reflash(ctx, via_shm=False)


def _scenario_servers_json(ctx: _Ctx) -> None:
    """Standalone D5: store initialized on image A → one good-rig upgrade
    flash of image B (the first boot into the target), then the
    preservation assertions."""
    _require_c_preconditions(ctx)
    _prepare_flash(ctx, ctx.image_b)
    pinned = _pinned(ctx)
    catalog.drop_e2e_marker(pinned)
    catalog.record_servers_json(pinned, ctx.state)
    catalog.record_generation(pinned, ctx.run)
    catalog.flash_e2e(pinned, ctx.image_b, assume_yes=ctx.yes, state=ctx.state)
    catalog.wait_for_device(ctx.dev)
    catalog.verify_upgraded(ctx.dev, ctx.run)
    catalog.require_staged_once(ctx.state)
    catalog.require_preservation_policy(
        ctx.dev, ctx.state, servers_json_preserved=ctx.servers_json_preserved
    )


def _scenario_stale_next_marker(ctx: _Ctx) -> None:
    """Standalone C5: initialized store on A → plant the stale marker, then
    one good-rig upgrade flash of B with the C5 assertions."""
    _require_c_preconditions(ctx)
    _prepare_flash(ctx, ctx.image_b)
    pinned = _pinned(ctx)
    catalog.drop_e2e_marker(pinned)
    catalog.plant_stale_next_marker(pinned)
    catalog.record_generation(pinned, ctx.run)
    catalog.flash_e2e(pinned, ctx.image_b, assume_yes=ctx.yes, state=ctx.state)
    catalog.wait_for_device(ctx.dev)
    catalog.verify_upgraded(ctx.dev, ctx.run)
    catalog.require_stale_next_gone(ctx.dev)


def _group_c(ctx: _Ctx) -> None:
    for sid in _GROUP_C_ORDER:
        _DRIVERS[sid](ctx)


def _group_d(ctx: _Ctx) -> None:
    """Precondition: initialized store on image A. D5 → D4 → D1 in two
    flashes: the upgrade to B (D4 rides its capture), then the /dev/shm
    same-version re-flash of B."""
    _scenario_servers_json(ctx)  # includes the staged-once (D4) assertion
    _scenario_shm_local_file(ctx)  # now running B: same-version /dev/shm re-flash of B


def _all(ctx: _Ctx) -> None:
    _group_a(ctx)
    _group_b(ctx)
    _group_c(ctx)


_DRIVERS: dict[str, Callable[[_Ctx], None]] = {
    "wrong-key-signature": _scenario_wrong_key_signature,
    "unsigned-feed": _scenario_unsigned_feed,
    "untrusted-key-name": _scenario_untrusted_key_name,
    "corrupt-tarball": _scenario_corrupt_tarball,
    "download-stall": _scenario_download_stall,
    "blank-data-partition": _scenario_blank_data_partition,
    "corrupt-fs-metadata": _scenario_corrupt_fs_metadata,
    "store-remnants": _scenario_store_remnants,
    "missing-store-db": _scenario_missing_store_db,
    "unmounted-store": _scenario_unmounted_store,
    "cache-swap-retry": _scenario_cache_swap_retry,
    "unreachable-rig": _scenario_unreachable_rig,
    "malformed-index": _scenario_malformed_index,
    "wrong-cache-key": _scenario_wrong_cache_key,
    "stale-next-marker": _scenario_stale_next_marker,
    "same-version-reflash": _scenario_same_version_reflash,
    "full-store": _scenario_full_store,
    "shm-local-file": _scenario_shm_local_file,
    "staged-once": _scenario_staged_once,
    "servers-json": _scenario_servers_json,
    "a": _group_a,
    "b": _group_b,
    "c": _group_c,
    "d": _group_d,
    "all": _all,
}


@dataclass
class E2eSysupgradeFaults:
    device: str  # IP or host of the target Deck
    image_a: Path  # baseline firmware tar; the init-path family flashes it
    image_b: Path  # target firmware tar; the upgrade-path family flashes it
    scenario: Scenario = "all"  # a single id, a group (a/b/c/d), or all
    serve_ip: str | None = None  # device-facing rig address (default: auto-detected)
    serve_port: int = 8083  # rig HTTP port
    # D5 asserts servers.json survives byte-identical; --no-servers-json-preserved
    # downgrades to observe-only for images predating the conffile registration
    servers_json_preserved: bool = True
    yes: bool = False  # skip the confirm prompts (cleardown + each flash)
    dry_run: bool = False  # run read-only checks; log mutations without executing

    def run(
        self,
        dev: Device | None = None,
        backend: Nix | None = None,
        make_device: Callable[[str], Device] = Device,
        make_server: Callable[[Path], rig.RigServer] | None = None,
    ) -> None:
        if self.dry_run:
            dry_run.set(True)
        dev = dev or Device(self.device)
        backend = backend or nix.real()
        run = catalog.E2eRun(image_a=Image(self.image_a), image_b=Image(self.image_b))
        state = catalog.FaultsState()
        prov = catalog.Provisioning()

        console.header("Sysupgrade e2e faults")
        dev.print()
        run.image_a.print()
        run.image_b.print()

        catalog.ensure_device_reachable(dev)
        catalog.capture_server_registry(dev, prov)
        catalog.capture_nix_conf(dev, prov)
        catalog.validate_firmware_image(run.image_a, device_target=dev.target)
        catalog.validate_firmware_image(run.image_b, device_target=dev.target)
        catalog.validate_e2e_inputs(run)
        catalog.build_e2e_artifacts(backend, run)
        catalog.build_nix_cli(backend, prov)

        serve_ip = self.serve_ip or default_serve_ip(dev.host)
        server_factory = make_server or (lambda root: rig.RigServer(root, port=self.serve_port))
        # The workdir holds the rig cache and the private signing key;
        # neither may outlive the run.
        with tempfile.TemporaryDirectory(
            prefix="sysupgrade-e2e-faults.", ignore_cleanup_errors=True
        ) as tmp:
            workdir = Path(tmp)
            serve_root = workdir / "serve"
            serve_root.mkdir()
            with server_factory(serve_root) as server:
                base_url = f"http://{serve_ip}:{server.port}"
                catalog.assemble_rig(backend, run, workdir=workdir, base_url=base_url)
                ctx = _Ctx(
                    dev=dev,
                    nix=backend,
                    run=run,
                    prov=prov,
                    state=state,
                    server=server,
                    image_a=run.image_a,
                    image_b=run.image_b,
                    make_device=make_device,
                    yes=self.yes,
                    servers_json_preserved=self.servers_json_preserved,
                )
                failed = False
                try:
                    catalog.preflight_rig(dev, run)
                    _DRIVERS[self.scenario](ctx)
                except BaseException:
                    failed = True
                    raise
                finally:
                    if run.device_mutated:
                        # A failure inside a quiesced window leaves the mDNS
                        # name dead until a reboot — clean up through the
                        # numeric handle whenever one exists, or every stage
                        # below no-ops against a name that cannot resolve.
                        cleanup = ctx.quiesced_pin or _pinned(ctx)
                        _best_effort(lambda: catalog.cleanup_e2e_marker(cleanup))
                        # leaving the rig registration behind must fail the
                        # run, not degrade to a log line
                        _restore_step(
                            failed=failed,
                            action=lambda: catalog.restore_server_registry(cleanup, prov),
                        )
                        _restore_step(
                            failed=failed,
                            action=lambda: catalog.restore_nix_conf(cleanup, prov),
                        )
                        _best_effort(lambda: catalog.sweep_uploaded_images(cleanup, run))
                        _best_effort(lambda: catalog.sweep_shm_upload(cleanup, run.image_a))
                        _best_effort(lambda: catalog.sweep_shm_upload(cleanup, run.image_b))
                        _best_effort(lambda: catalog.cleanup_remote_artifacts(cleanup, prov))
                        _best_effort(lambda: catalog.start_compositor(cleanup))
                        _best_effort(lambda: _restore_rig(ctx))


@entrypoint
def main(args: E2eSysupgradeFaults) -> None:
    args.run()


if __name__ == "__main__":
    main()

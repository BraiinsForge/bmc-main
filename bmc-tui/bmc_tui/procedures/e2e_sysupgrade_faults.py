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

This module lands the skeleton: the pinned all-suite order, the scenario
dispatch registry, the shared preamble/cleanup, and the rig-restore and
dry-run tamper seams. The scenario bodies are stubs until Tasks 13-15,
so the procedure is deliberately kept off the CLI.
"""

import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from bmc_tui import catalog, console, nix, rig
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.nix import Nix
from bmc_tui.stage import dry_run, entrypoint

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
    "shm-local-file",  # D1
    "staged-once",  # D4
    "servers-json",  # D5
]

# The pinned `all` order: fault scenarios grouped by init/upgrade family,
# with cheaper faults first so a common regression surfaces early.
# cache-swap-retry (C1) fuses stale-next-marker (C5) + servers-json (D5)
# onto its retry flash; same-version-reflash (C6) fuses shm-local-file
# (D1); the fused ids therefore have drivers but do not appear here.
# `good-init` is the clean init flash that closes group A — a flash step,
# not a scenario driver.
SUITE_ORDER = (
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
    "cache-swap-retry",  # fuses stale-next-marker + servers-json on retry
    "same-version-reflash",  # fuses shm-local-file (staged via /dev/shm)
)


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


def _unimplemented(ctx: _Ctx) -> None:
    raise NotImplementedError("implemented in a later task")


def _prepare_flash(ctx: _Ctx, image: Image) -> None:
    """Everything a flash attempt needs, idempotent per attempt: CLI +
    registration, RAM headroom, pinned address, verified upload, trusted
    keys. Ordered like the happy path's scenario drivers."""
    ctx.run.device_mutated = True
    catalog.push_nix_cli(ctx.dev, ctx.prov)
    catalog.register_rig(ctx.dev, ctx.run)
    catalog.ensure_memory(ctx.dev, image.size + image.rootfs_size + _FLASH_HEADROOM)
    catalog.pin_device_address(ctx.dev, ctx.run)
    pinned = _pinned(ctx)
    catalog.upload_firmware(ctx.dev, image)
    catalog.require_uploaded(pinned, image)
    catalog.trust_image_keys(pinned, image)


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
    device behavior, not cleanup: A2/A3 abort before any bytes are
    fetched, and A1/A4's downloaded tarball is deleted by bmc-nix
    itself on signature rejection (store.rs). A5 passes
    artifact_deleted=False — the stall/download-error paths return
    WITHOUT deleting the partial file, so only the finally-sweep (which
    always runs, pass or fail) cleans it there."""
    catalog.require_store_absent(ctx.dev)
    _prepare_flash(ctx, ctx.image_a)
    pinned = _pinned(ctx)
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
        catalog.sweep_download_artifact(pinned)
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
    _prepare_flash(ctx, ctx.image_a)
    catalog.flash_e2e(_pinned(ctx), ctx.image_a, assume_yes=ctx.yes, state=ctx.state)
    catalog.wait_for_device(ctx.dev)
    catalog.verify_initialized(ctx.dev, ctx.run)
    catalog.require_staged_once(ctx.state)


def _group_a(ctx: _Ctx) -> None:
    catalog.clear_nix_store(ctx.dev, assume_yes=ctx.yes)
    ctx.run.device_mutated = True
    for sid in (
        "unsigned-feed",
        "untrusted-key-name",
        "wrong-key-signature",
        "corrupt-tarball",
        "download-stall",
    ):
        _DRIVERS[sid](ctx)
    _flash_good_init(ctx)


def _group_b(ctx: _Ctx) -> None:
    for sid in (
        "store-remnants",
        "missing-store-db",
        "unmounted-store",
        "blank-data-partition",
        "corrupt-fs-metadata",
    ):
        _DRIVERS[sid](ctx)


def _group_c(ctx: _Ctx) -> None:
    # the fused ids (stale-next-marker, shm-local-file, servers-json) have
    # drivers but never run standalone here
    for sid in (
        "unreachable-rig",
        "malformed-index",
        "wrong-cache-key",
        "cache-swap-retry",
        "same-version-reflash",
    ):
        _DRIVERS[sid](ctx)


def _group_d(ctx: _Ctx) -> None:
    """The standalone D group — its scenarios are fused into groups A-C on
    the `all` run, so this only runs when invoked directly. Implemented in
    a later task."""
    raise NotImplementedError("implemented in a later task")


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
    "blank-data-partition": _unimplemented,
    "corrupt-fs-metadata": _unimplemented,
    "store-remnants": _unimplemented,
    "missing-store-db": _unimplemented,
    "unmounted-store": _unimplemented,
    "cache-swap-retry": _unimplemented,
    "unreachable-rig": _unimplemented,
    "malformed-index": _unimplemented,
    "wrong-cache-key": _unimplemented,
    "stale-next-marker": _unimplemented,
    "same-version-reflash": _unimplemented,
    "shm-local-file": _unimplemented,
    "staged-once": _unimplemented,
    "servers-json": _unimplemented,
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
    servers_json_preserved: bool = False  # device keeps servers.json across flashes
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
        catalog.validate_firmware_image(run.image_a, device_target=dev.target)
        catalog.validate_firmware_image(run.image_b, device_target=dev.target)
        catalog.validate_e2e_inputs(run)
        catalog.build_e2e_artifacts(backend, run)
        catalog.build_nix_cli(backend, prov)

        serve_ip = self.serve_ip or rig.default_serve_ip(dev.host)
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
                try:
                    catalog.preflight_rig(dev, run)
                    _DRIVERS[self.scenario](ctx)
                finally:
                    if run.device_mutated:
                        _best_effort(lambda: catalog.cleanup_e2e_marker(dev))
                        _best_effort(lambda: catalog.cleanup_server_registry(dev))
                        _best_effort(lambda: catalog.sweep_uploaded_images(dev, run))
                        _best_effort(lambda: catalog.sweep_shm_upload(dev, run.image_a))
                        _best_effort(lambda: catalog.sweep_shm_upload(dev, run.image_b))
                        _best_effort(lambda: catalog.cleanup_remote_artifacts(dev, prov))
                        _best_effort(lambda: _restore_rig(ctx))
                        if not dry_run.get():
                            console.kv(
                                "note",
                                "the rig's extra-substituters/extra-trusted-public-keys lines "
                                "persist in /etc/nix/nix.conf (preserved conffile) — remove "
                                "them when done",
                            )


@entrypoint
def main(args: E2eSysupgradeFaults) -> None:
    args.run()


if __name__ == "__main__":
    main()

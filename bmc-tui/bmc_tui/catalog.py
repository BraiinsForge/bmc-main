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

"""Reusable deploy stages, composed by the procedure scripts.

Each stage is a guarded function (see bmc_tui.stage). Device access goes through
the read/run seam, so `--dry-run` skips mutations while read-only checks still
run. The authoritative firmware compatibility check runs on the device during
sysupgrade; this catalog only fails fast on the obvious local problems.
"""

import difflib
import json
import os
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, NewType

from bmc_tui import console, nix_progress, rig
from bmc_tui.device import Device, RemotePath
from bmc_tui.image import Image
from bmc_tui.nix import Attr, Built, Nix, Pkg, StorePath
from bmc_tui.stage import Abort, done_if, dry_run, ensure, require, stage

_PROFILE_DIR = "/nix/var/nix/gcroots/profiles/bmc"
# Probe and invoke the CLI at the profile we deploy into, not
# via the /run/current-profile symlink — the symlink only flips
# to the bmc profile at boot, so right after a bootstrap
# it can disagree with what we just registered.
_NIX_CLI = f"{_PROFILE_DIR}/current/bin/bmc-nix-cli"

_NIX_CONF = "/etc/nix/nix.conf"

# Device-side nix store: a directory on the data partition, bind-mounted at /nix
# so the read-only rootfs gains a writable store. Matches the init tarball layout.
_NIX_STORE = "/nix"
_WIDGET_DIR = "/run/current-profile/lib/bmc-widgets"
_COMPOSITOR = "bmc-openwrt"
_ORCHESTRATOR = "bmc-nix-service-orchestrator"
_ORCHESTRATOR_TIMEOUT = 30  # seconds; it holds the profile lock for its whole run
_NIX_BACKING = "/mnt/data/nix"
_INIT_TARBALL = Attr(".#init-tarball-armv7")
_NIX_CLI_ATTR = Attr(".#bmc-nix-cli-armv7-release")
# Host-native CLI used to sign the rig's init tarballs; the device CLI is
# cross-compiled for ARM and cannot run on the harness host.
_HOST_CLI_ATTR = Attr(".#bmc-nix-cli")
# Run-unique device paths for the pushed CLI and tarball: Device.push
# truncates via `cat >`, so a constant name would let concurrent
# harness runs upload, read, and clean up the same file.
_REMOTE_CLI = RemotePath(f"/tmp/bmc-nix-cli.deck-init.{os.getpid()}")


@stage("Device reachable")
def ensure_device_reachable(dev: Device) -> None:
    require(
        dev.reachable,
        f"{dev.host} is unreachable — power-cycle the Deck and check the network",
    )


def running_binary(dev: Device, process: str) -> StorePath | None:
    """Store path of a running process's executable, None when it is not up.

    Callers that collect measurements should re-read this afterwards:
    an equal path proves the whole window belongs to one build.
    """
    found = dev.read(
        f"p=$(pidof {process} | cut -d' ' -f1); readlink -f /proc/$p/exe 2>/dev/null || true"
    ).strip()
    return StorePath(found) if found else None


@stage("Profiling build")
def ensure_profiling_build(dev: Device, process: str, marker: str) -> str:
    """Prove instrumentation from the running binary rather than from its log.

    A log outlives the build that wrote it, so a debug-then-release redeploy
    leaves stale `mesh::profile` lines proving a build that is gone.
    `marker` is a literal only the profiling build contains.
    """
    path = running_binary(dev, process)
    if path is None:
        raise Abort(f"{process} is not running — deploy it before measuring")
    found = dev.read(f"grep -aq {marker} {path} && echo yes || echo no").strip()
    require(
        found == "yes",
        f"{process} carries no profiling instrumentation — redeploy with "
        f"`deck deploy --device {dev.host} --profile debug`",
    )
    return path.rsplit("/", 1)[-1]


def deployed_widget_path(dev: Device, widget: str) -> StorePath | None:
    """Store path of a deployed widget package, None when it is absent."""
    resolved = dev.read(f"readlink -f {_WIDGET_DIR}/{widget}")
    return StorePath(resolved.split("/lib/", 1)[0]) if resolved else None


@stage("Deployed build")
def check_deployed_build(backend: Nix, dev: Device, attr: Attr, widget: str) -> str:
    """Compare what this tree builds against what the device runs.

    Store paths are input-addressed, so equality means one derivation produced
    both — the device is running this tree's build, which is what a measurement
    run depends on and what a firmware version cannot tell you. Answers "should
    I redeploy first", so it warns and asks rather than blocking. Without a TTY
    to ask on, it proceeds loudly.
    """
    installed = deployed_widget_path(dev, widget)
    if installed is None:
        raise Abort(f"the {widget} widget is not deployed on {dev.host}")
    expected = backend.out_path(attr)
    if expected == installed:
        return installed.rsplit("/", 1)[-1]
    # Same-width labels so the two hashes line up for character comparison.
    console.warn(f"the deployed {widget} widget is not what this tree builds")
    console.kv(" - on deck", installed.rsplit("/", 1)[-1])
    console.kv(" - in repo", expected.rsplit("/", 1)[-1])
    console.blank()
    if not sys.stdin.isatty():
        return "MISMATCH, unacknowledged (no TTY)"
    require(
        console.confirm("Continue against a build this tree did not produce?"),
        "deployed build mismatch not acknowledged",
    )
    return "mismatch acknowledged"


@stage("Validate firmware image")
def validate_firmware_image(image: Image, *, device_target: str) -> None:
    require(image.path.is_file(), f"image not found: {console.lit(image.path)}")
    name = console.lit(image.path.name)
    require(image.is_sysupgrade, f"not a Deck sysupgrade image: {name}")
    token = device_target.replace("/", "_")  # stm32mp15/ii3 -> stm32mp15_ii3
    require(
        token in (image.sysupgrade_dir or ""),
        f"wrong board family: {name} is not for {console.lit(device_target)}",
    )


@stage("Memory headroom")
def ensure_memory(dev: Device, need: int) -> str:
    available = _mem_available(dev)
    if available < need:
        available = _offer_stale_firmware_cleanup(dev, available)
    require(
        available >= need,
        f"need {console.human_size(need)} free RAM, only {console.human_size(available)} available",
    )
    return f"{console.lit(console.human_size(available))} RAM available"


def _offer_stale_firmware_cleanup(dev: Device, available: int) -> int:
    """Short on RAM, so offer to drop firmware tars left in tmpfs by a run that
    never reached its own cleanup — a killed or crashed one. Returns what is
    free afterwards.

    Only `*.tar` is offered: `/tmp` on the device holds live system state
    (resolv.conf, leases, logs), so a broader sweep could break the running
    system. The listing prints before the prompt, so a non-interactive run
    still reports what is occupying the space it lacks.
    """
    stale = _tmp_firmware(dev)
    if not stale:
        return available
    console.warn(f"{len(stale)} firmware tar(s) left in /tmp — tmpfs, so they hold RAM")

    # cmd_output, not kv: this list explains the warning above it,
    # and kv would put it on stdout where a redirect could separate the two.
    console.cmd_output("\n".join(stale))

    if not console.confirm("remove them?"):
        return available

    dev.run("rm -f " + " ".join(shlex.quote(p) for p in stale))
    return _mem_available(dev)


def _tmp_firmware(dev: Device) -> list[str]:
    """Firmware tars currently in the device's /tmp, newest listing order."""
    listed = dev.read("ls -1 /tmp/*.tar 2>/dev/null")
    return [line for line in listed.splitlines() if line.strip()]


@stage("Upload firmware")
def upload_firmware(dev: Device, image: Image) -> str:
    """Upload the firmware; a matching on-device sha256 makes a re-run a no-op."""

    done_if(_remote_sha(dev, image.remote_path) == image.sha256)
    dev.push(image.path, image.remote_path)
    if dry_run.get():
        return f"→ {console.lit(image.remote_path)}"
    # A short/corrupt upload would otherwise be flashed blind under `sysupgrade
    # -F`; verifying the bytes on the device before we ever flash prevents it.
    require(
        _remote_sha(dev, image.remote_path) == image.sha256,
        f"upload corrupted: {console.lit(image.path.name)} checksum mismatch on device",
    )
    return f"→ {console.lit(image.remote_path)} (sha256 verified)"


@stage("Sysupgrade")
def sysupgrade(
    dev: Device,
    image: Image,
    *,
    force: bool = False,
    assume_yes: bool = False,
    skip_nix: bool = False,
) -> str:
    done_if(dev.version == image.version)
    require(
        assume_yes
        or dry_run.get()
        or console.confirm(
            f"Flash {console.lit(image.version)} to {console.lit(dev.host)}? "
            "The device will reboot."
        ),
        "flash declined — pass --yes to skip the prompt",
    )
    flag = "-F " if force else ""
    # BOS_NIX_SKIP=1 skips the firmware's Nix-profile staging.
    # The escape hatch when an old device nix.conf (build-users-group=nixbld) breaks it.
    env = "BOS_NIX_SKIP=1 " if skip_nix else ""
    nix_progress.stream_flash(dev, f"{env}sysupgrade {flag}{shlex.quote(image.remote_path)}")
    return f"{console.lit(image.version)} → reboot"


@stage("Clear firmware from /tmp")
def cleanup_firmware(dev: Device, image: Image) -> str:
    """Remove this image's tar from /tmp (tmpfs), before the upload and after the
    flash. A successful flash reboots and wipes it; a killed or crashed run leaves
    it in RAM, where it eats the headroom the next run needs.

    Clearing it up front means the upload is never skipped by the checksum
    shortcut in `upload_firmware`, so every flash sends bytes it just verified."""
    dev.run(f"rm -f {shlex.quote(image.remote_path)}")
    return console.lit(image.remote_path)


@stage("Wait for device")
def wait_for_device(dev: Device, *, timeout: int = 180) -> None:
    require(
        _wait_reachable(dev, timeout),
        f"{dev.host} did not return within {timeout}s — power-cycle it",
    )


@stage("Verify post-upgrade")
def verify_post_upgrade(dev: Device, *, expect: str) -> str:
    version = dev.version
    require(version == expect, f"firmware is {version}, expected {expect}")
    require("/nix" in dev.read("mount"), "/nix is not mounted after the upgrade")
    require(
        "experimental-features" in dev.read(f"cat {_NIX_CONF} 2>/dev/null"),
        "nix.conf lost its experimental-features after the upgrade",
    )
    return f"running {console.lit(version)}, {console.lit('/nix')} mounted, nix.conf intact"


# ── nix package deploy ────────────────────────────────────────────────────────

_DECK_PACKAGES = ".#deck-packages"


def package_prefix(profile: str) -> str:
    """Flake attr root for a build `profile`.

    `release` deploys `.#deck-packages.<name>`; any other profile (e.g.
    `debug`) deploys the parallel `.#deck-packages-<profile>.<name>` set, which
    is the same packages built with the compositor + wasm-host `profiling`
    feature on (the mesh::profile timing/memory channel).
    """
    return _DECK_PACKAGES if profile == "release" else f"{_DECK_PACKAGES}-{profile}"


@dataclass
class Deployment:
    """Mutable carrier threaded through the deploy stages."""

    attrs: list[Attr]  # flake attrs to deploy; empty → discover core, bmc-nix-cli + all widgets
    prefix: str = _DECK_PACKAGES  # attr root for the build profile (see package_prefix)
    resolved: list[Pkg] = field(default_factory=list)
    built: list[Built] = field(default_factory=list)


@stage("bmc-nix-cli present")
def ensure_nix_cli(nix: Nix, dev: Device) -> None:
    def present() -> bool:
        return "ok" in dev.read(f"test -x {_NIX_CLI} && echo ok || true")

    def bootstrap() -> None:
        [built] = nix.build([nix.resolve(Attr(".#deck-packages.bmc-nix-cli"))])
        nix.copy([built.store_path], dev.copy_dest)
        dev.run(_register_cmd([built], cli=f"{built.store_path}/bin/bmc-nix-cli"))

    ensure(present, bootstrap, "bmc-nix-cli bootstrap did not take")


def _unknown_package_hint(attr: str, packages: list[str], prefix: str) -> str:
    """Clean 'does not exist' hint, suggesting the closest deck package."""
    leaf = attr.rsplit(".", 1)[-1]
    prefixed = f"widget-{leaf}"
    if prefixed in packages:
        guess = prefixed
    else:
        matches = difflib.get_close_matches(leaf, packages, n=1, cutoff=0.5)
        guess = matches[0] if matches else None
    suffix = f" — did you mean {console.lit(f'{prefix}.{guess}')}?" if guess else ""
    return f"package {console.lit(attr)} does not exist{suffix}"


def _qualify(attr: str, prefix: str) -> Attr:
    """Expand a bare package name to its `prefix.` attr (profile-aware)."""
    return Attr(attr if "#" in attr else f"{prefix}.{attr}")


@stage("Resolve packages")
def resolve_packages(nix: Nix, plan: Deployment) -> str:
    if not plan.attrs:
        names = ["core", "bmc-nix-cli", *nix.discover_widgets()]
        plan.attrs = [Attr(f"{plan.prefix}.{name}") for name in names]
    plan.attrs = [_qualify(a, plan.prefix) for a in plan.attrs]
    resolved: list[Pkg] = []
    for attr in plan.attrs:
        try:
            resolved.append(nix.resolve(attr))
        except subprocess.CalledProcessError:
            raise Abort(_unknown_package_hint(attr, nix.list_packages(), plan.prefix)) from None
    plan.resolved = resolved
    return ", ".join(console.lit(pkg.name) for pkg in plan.resolved)


@stage("Build packages")
def build_packages(nix: Nix, plan: Deployment) -> str:
    try:
        plan.built = nix.build(plan.resolved)
    except subprocess.CalledProcessError as e:
        raise Abort(f"nix build failed (exit {e.returncode}); see the nix output above") from None
    return f"built {console.lit(len(plan.built))} package(s)"


@stage("Copy closures")
def copy_closures(nix: Nix, dev: Device, plan: Deployment) -> str:
    nix.copy([b.store_path for b in plan.built], dev.copy_dest)
    return f"{console.lit(len(plan.built))} closure(s) → {console.lit(dev.host)}"


# Developers carry the pre-rename `flip-clock` package, which shares store
# paths with `widget-flip-clock`; left in place it makes add-packages fail on a
# symlink conflict. Drop it only when its successor is in this deploy, and treat
# it as advisory — a device without it is already fine — so a failure here (e.g.
# it was never installed) never blocks a deploy.
_LEGACY_FLIP_CLOCK = "flip-clock"
_WIDGET_FLIP_CLOCK = "widget-flip-clock"


@stage("Drop legacy flip-clock")
def remove_legacy_flip_clock(dev: Device, plan: Deployment) -> str:
    done_if(all(b.name != _WIDGET_FLIP_CLOCK for b in plan.built))
    cmd = (
        f"PATH=/run/current-profile/bin:$PATH {shlex.quote(_NIX_CLI)} "
        f"remove-packages --name {shlex.quote(_LEGACY_FLIP_CLOCK)}"
    )
    try:
        dev.run(cmd)
    except subprocess.SubprocessError:
        return f"{_LEGACY_FLIP_CLOCK} not present (ignored)"
    return f"{_LEGACY_FLIP_CLOCK} removed"


@stage("Register in bmc profile")
def register_packages(dev: Device, plan: Deployment) -> str:
    out = dev.run(_register_cmd(plan.built))
    names = ", ".join(f"{console.lit(b.name)} {b.version}" for b in plan.built)
    generation = _generation_number(out)
    return f"{names} → generation {console.lit(generation)}" if generation else names


Pid = NewType("Pid", str)


def compositor_pid(dev: Device) -> Pid | None:
    """PID of the running compositor, None when it is down."""
    found = dev.read(f"pidof {_COMPOSITOR} | cut -d' ' -f1").strip()
    return Pid(found) if found else None


def _await_orchestrator(dev: Device, timeout: int = _ORCHESTRATOR_TIMEOUT) -> None:
    """Block while the post-activation service reconciliation is still running.

    It is spawned detached and waits on the profile lock.
    That lets it bounce the compositor after `add-packages` has returned,
    so sampling before it settles would miss the restart it is about to make.
    """
    dev.read(
        f"i=0; while pidof {_ORCHESTRATOR} >/dev/null 2>&1 && [ $i -lt {timeout} ]; "
        "do sleep 1; i=$((i+1)); done"
    )


@stage("Restart compositor")
def restart_compositor(dev: Device, *, old_pid: Pid | None = None, skip: bool = False) -> str:
    """Restart the compositor so it reloads the widget set.

    The orchestrator only reloads it when its init script changed.
    That happens for a compositor package change, not a widget-only one.
    Pass `old_pid`, sampled ahead of the activation, to tell those apart:
    a changed pid means the reload already happened.

    Deliberately not a question. The prompt it replaces defaulted to no,
    answering itself that way whenever stdin was not a TTY — so a deploy
    could report success having loaded nothing.
    `skip` is the explicit opt-out for leaving a running display alone.
    """
    if dry_run.get():
        return "skipped (dry-run)"

    if skip:
        return "skipped on request — widgets load on the compositor's next start"

    if old_pid is not None:
        _await_orchestrator(dev)
        now = compositor_pid(dev)
        if now is not None and now != old_pid:
            return f"already restarted by the service orchestrator (pid {now})"

    dev.run("/etc/init.d/bmc-compositor restart")
    return "restarted"


def _generation_number(register_stdout: str | None) -> str | None:
    """Generation number from `add-packages` stdout's last line; None if absent."""

    if not register_stdout:
        return None
    leaf = register_stdout.splitlines()[-1].rsplit("/", 1)[-1]
    return leaf.removesuffix("-link") if leaf.endswith("-link") else None


def _register_cmd(built: list[Built], *, cli: str = _NIX_CLI) -> str:
    args = [cli, "add-packages", "--profile-dir", _PROFILE_DIR]
    for b in built:
        args += ["--name", b.name, "--version", b.version, "--store-path", b.store_path]
    inner = " ".join(shlex.quote(a) for a in args)
    return f"PATH=/run/current-profile/bin:$PATH {inner}"


# ── device init ───────────────────────────────────────────────────────────────


@dataclass
class Provisioning:
    """Mutable carrier threaded through the init stages."""

    tarball: Path | None = None  # the built init tarball, located by build_init_tarball
    profile_path: str | None = None  # promoted profile path, from the tarball's metadata.json
    cli: Path | None = None  # host path of the built bmc-nix-cli
    remote_tarball: RemotePath | None = None  # device path of the pushed tarball


@stage("Build init tarball")
def build_init_tarball(nix: Nix, plan: Provisioning) -> str:
    """Build the init tarball and validate its metadata.json contract."""

    out = Path(nix.build_out(_INIT_TARBALL))
    meta_path = out / "metadata.json"
    require(meta_path.is_file(), f"no metadata.json in {console.lit(out)}")
    meta = json.loads(meta_path.read_text())
    missing = [k for k in ("bos_version", "profile_path", "tarball_name") if not meta.get(k)]
    require(not missing, f"metadata.json misses: {', '.join(missing)}")
    name = meta["tarball_name"]
    require("/" not in name, f"tarball_name is not a basename: {console.lit(name)}")
    tarball = out / name
    require(tarball.is_file(), f"metadata.json names a missing archive: {console.lit(name)}")
    profile_path = meta["profile_path"]
    # normpath equality rejects "..", ".", doubled and trailing slashes,
    # so the prefix check can't be escaped via /nix/../elsewhere.
    require(
        profile_path == os.path.normpath(profile_path) and profile_path.startswith("/nix/"),
        f"profile_path is not a normalized path under /nix: {console.lit(profile_path)}",
    )
    plan.tarball = tarball
    plan.profile_path = profile_path
    size = console.human_size(tarball.stat().st_size)
    return f"{console.lit(name)} ({console.lit(size)})"


@stage("Build bmc-nix-cli")
def build_nix_cli(nix: Nix, plan: Provisioning) -> str:
    out = Path(nix.build_out(_NIX_CLI_ATTR))
    cli = out / "bin/bmc-nix-cli"
    require(cli.is_file(), f"no bin/bmc-nix-cli in {console.lit(out)}")
    plan.cli = cli
    return console.lit(cli.name)


@stage("Push bmc-nix-cli")
def push_nix_cli(dev: Device, plan: Provisioning) -> str:
    cli = plan.cli
    if cli is None:
        msg = "BUG: bmc-nix-cli was not built before the push stage"
        raise RuntimeError(msg)
    dev.push(cli, _REMOTE_CLI)
    dev.run(f"chmod +x {shlex.quote(_REMOTE_CLI)}")
    return f"→ {console.lit(_REMOTE_CLI)}"


@stage("Prepare data partition")
def prepare_data_partition(dev: Device) -> str:
    """Fsck/format/mount /mnt/data via the pushed CLI; a healthy running
    Deck makes this a mount-state no-op."""

    dev.run(f"{shlex.quote(_REMOTE_CLI)} prepare-data-partition")
    return f"{console.lit('/mnt/data')} ready"


@stage("Nix store absent")
def ensure_store_absent(dev: Device) -> str:
    """Refuse to reinitialise over a populated store; clear empty leftovers.

    Runs after prepare_data_partition, so the verdict is about the real
    data partition, and matches the CLI's is_initialized "any directory
    counts" semantics — the CLI can never turn this stage's "absent"
    verdict into a silent no-op.
    """

    if dry_run.get() and not dev.read("grep ' /mnt/data ' /proc/mounts || true"):
        return "store absence not verified (partition not prepared under dry-run)"

    remedy = f"ssh {dev.login} 'umount /nix 2>/dev/null; rm -rf /mnt/data/nix /nix'"
    mounted = bool(dev.read(f"grep ' {_NIX_STORE} ' /proc/mounts || true"))
    identical = bool(dev.read(f"[ {_NIX_STORE} -ef {_NIX_BACKING} ] && echo yes || true"))
    require(
        not mounted or identical,
        f"{console.lit(_NIX_STORE)} is mounted from a foreign source — clear it first: "
        f"{console.lit(remedy)}",
    )
    if not mounted:
        rootfs_listing = dev.read(f"[ -d {_NIX_STORE} ] && ls -A {_NIX_STORE} 2>/dev/null || true")
        require(
            not rootfs_listing,
            f"{console.lit(_NIX_STORE)} has rootfs content — clear it first: {console.lit(remedy)}",
        )
    backing_listing = dev.read(f"[ -d {_NIX_BACKING} ] && ls -A {_NIX_BACKING} 2>/dev/null || true")
    require(
        not backing_listing,
        f"{console.lit(_NIX_BACKING)} already populated — to reinitialise, first clear it: "
        f"{console.lit(remedy)}",
    )
    if dev.read(f"[ -d {_NIX_BACKING} ] && echo yes || true"):
        if mounted:
            # A bind mount would keep referencing the unlinked directory
            # inode after rmdir, so the post-init mount identity check
            # could never pass again.
            dev.run(f"umount {_NIX_STORE}")
        dev.run(f"rmdir {_NIX_BACKING}")
        return "removed an empty leftover store dir"
    return "store is clean"


@stage("Push init tarball")
def push_init_tarball(dev: Device, plan: Provisioning) -> str:
    tarball = plan.tarball
    if tarball is None:
        msg = "BUG: init tarball was not built before the push stage"
        raise RuntimeError(msg)
    # Recorded before the upload starts so a failed stream still leaves
    # the path known to the cleanup stage.
    remote = RemotePath(f"/mnt/data/{tarball.name}.deck-init.{os.getpid()}")
    plan.remote_tarball = remote
    dev.push(tarball, remote)
    return f"→ {console.lit(remote)}"


@stage("Initialise store")
def run_cli_init(dev: Device, plan: Provisioning) -> str:
    remote, profile_path = plan.remote_tarball, plan.profile_path
    if remote is None or profile_path is None:
        msg = "BUG: tarball was not pushed before the init stage"
        raise RuntimeError(msg)
    out = dev.run(
        f"{shlex.quote(_REMOTE_CLI)} init --tarball {shlex.quote(remote)} "
        f"--profile-path {shlex.quote(profile_path)}"
    )
    if out is not None:  # dry-run logs the command and returns None
        # A fresh init prints exactly the promoted profile path; a no-op
        # prints nothing. Both would be a bug here — the store was just
        # verified absent — so demand the exact fresh-init contract.
        require(
            out == profile_path,
            f"init printed {console.lit(out or '<nothing>')}, expected {console.lit(profile_path)}",
        )
    return f"promoted {console.lit(profile_path)}"


@stage("Activate profile")
def activate_profile(dev: Device, plan: Provisioning) -> str:
    """Bind /nix from the data partition and run generation 1's entrypoint."""

    profile_path = plan.profile_path
    if profile_path is None:
        msg = "BUG: tarball metadata was not read before activation"
        raise RuntimeError(msg)
    dev.run(f"{shlex.quote(_REMOTE_CLI)} mount")
    if not dry_run.get():
        # `mount` reports AlreadyMounted for ANY /nix mount without
        # checking the source; verify the identity ourselves, like the
        # firmware COMMAND does.
        require(
            bool(dev.read(f"[ {_NIX_STORE} -ef {_NIX_BACKING} ] && echo yes || true")),
            f"{console.lit(_NIX_STORE)} is not backed by {console.lit(_NIX_BACKING)} after mount",
        )
    entrypoint = f"{profile_path}/1-link/core/activation/entrypoint"
    dev.run(shlex.quote(entrypoint))
    return f"activated {console.lit('generation 1')}"


@stage("Clean up pushed files")
def cleanup_remote_artifacts(dev: Device, plan: Provisioning) -> str:
    """Remove the pushed CLI and tarball; runs also after a failed stage."""

    paths = [p for p in (plan.remote_tarball, _REMOTE_CLI) if p]
    dev.run(f"rm -f {' '.join(shlex.quote(p) for p in paths)}")
    return ", ".join(console.lit(p) for p in paths)


# ── sysupgrade e2e ────────────────────────────────────────────────────────────

_E2E_ARTIFACTS_FILE = "nix/e2e-artifacts.nix"
_E2E_ATTRS = ["index-a", "tarball-a", "index-b", "tarball-b"]
_SERVERS_JSON = "/etc/nix-upgrade/servers.json"
_E2E_MARKER = f"{_NIX_BACKING}/.sysupgrade-e2e-marker"
_BUMPED_PACKAGE = "bmc-nix-cli"
_NIX_PAYLOAD_MEMBERS = ("bmc-nix-cli", "servers.json.default")


@dataclass
class E2eRig:
    """The assembled rig's device-facing coordinates."""

    base_url: str
    feed_url: str
    cache_url: str
    cache_public_key: str
    preflight_urls: list[str]
    serve_root: Path
    cache: Path
    secret: Path
    host_cli: str


@dataclass
class E2eRun:
    """Mutable carrier threaded through the e2e-sysupgrade stages."""

    image_a: Image  # baseline firmware; scenario A flashes it over a cleared store
    image_b: Image  # target firmware; scenario B upgrades A's store with it
    variant_a: rig.Variant | None = None
    variant_b: rig.Variant | None = None
    rig: E2eRig | None = None
    pinned_host: str | None = None  # numeric address for the cleardown/flash window
    bumped_path: str | None = None  # variant B's bumped store path, from index B
    generation_before: str | None = None  # scenario B's pre-flash current generation
    device_mutated: bool = False  # set before the first device mutation; gates the sweep


@stage("Validate e2e images")
def validate_e2e_inputs(run: E2eRun) -> str:
    """Two differing versions are load-bearing: an index identical to the
    installed set yields an empty upgrade plan and never invokes nix-store.
    Both images must be Nix-era — a legacy image would pass every generic
    check and still reach the destructive cleardown."""

    a, b = run.image_a.version, run.image_b.version
    require(a != b, f"images A and B carry the same firmware version {console.lit(a)}")
    for image in (run.image_a, run.image_b):
        _require_nix_era(image)
    return f"A {console.lit(a)}, B {console.lit(b)}"


def _require_nix_era(image: Image) -> None:
    """COMMAND only enters the Nix flow when it finds its two payload
    members in the tar."""

    members = set(image.members())
    missing = [
        member
        for member in _NIX_PAYLOAD_MEMBERS
        if f"{image.sysupgrade_dir}/{member}" not in members
    ]
    require(
        not missing,
        f"{console.lit(image.path.name)} is not Nix-era — payload member(s) missing: "
        + ", ".join(missing),
    )


@stage("Build e2e artifacts")
def build_e2e_artifacts(nix: Nix, run: E2eRun) -> str:
    """Build both variants' index + tarball in one nix invocation (one
    consistent evaluation of the worktree), for the images' exact
    versions — the invariant the CLI cannot check itself: tarball
    metadata == feed entry == flashed firmware version."""

    args = {"bosVersionA": run.image_a.version, "bosVersionB": run.image_b.version}
    outs = dict(
        zip(
            _E2E_ATTRS,
            nix.build_file(_E2E_ARTIFACTS_FILE, [Attr(a) for a in _E2E_ATTRS], args),
            strict=True,
        )
    )
    run.variant_a = _variant(run.image_a.version, outs["index-a"], outs["tarball-a"])
    run.variant_b = _variant(run.image_b.version, outs["index-b"], outs["tarball-b"])
    return f"{console.lit(run.variant_a.tarball.name)}, {console.lit(run.variant_b.tarball.name)}"


def _variant(bos_version: str, index_out: str, tarball_out: str) -> rig.Variant:
    """Validate a variant's tarball metadata and locate its archive."""

    meta_path = Path(tarball_out) / "metadata.json"
    require(meta_path.is_file(), f"no metadata.json in {console.lit(tarball_out)}")
    meta = json.loads(meta_path.read_text())
    built_for = meta.get("bos_version") or "<unset>"
    require(
        built_for == bos_version,
        f"artifact carries {console.lit(built_for)}, built for {console.lit(bos_version)}",
    )
    name = meta.get("tarball_name") or ""
    tarball = Path(tarball_out) / name
    require(
        bool(name) and "/" not in name and tarball.is_file(),
        f"metadata.json names a missing archive: {console.lit(name)}",
    )
    profile_path = meta.get("profile_path") or ""
    require(
        profile_path == os.path.normpath(profile_path) and profile_path.startswith("/nix/"),
        f"profile_path is not a normalized path under /nix: {console.lit(profile_path)}",
    )
    return rig.Variant(
        bos_version=bos_version,
        profile_path=profile_path,
        index=Path(index_out),
        tarball=tarball,
    )


@stage("Assemble rig")
def assemble_rig(nix: Nix, run: E2eRun, *, workdir: Path, base_url: str) -> str:
    """Generate the signing key, sign each variant's init tarball with the
    host-built CLI, then write the serve tree and the signed cache — the
    key must exist before the feed is written, because the feed now
    carries the tarball signatures the device verifies (BDK-376)."""

    a, b = run.variant_a, run.variant_b
    if a is None or b is None:
        msg = "BUG: e2e artifacts were not built before rig assembly"
        raise RuntimeError(msg)
    workdir.mkdir(parents=True, exist_ok=True)
    secret = workdir / "cache-key.secret"
    public = rig.generate_cache_key(nix, secret)
    host_cli = nix.build_out(_HOST_CLI_ATTR)
    a = rig.sign_variant(host_cli, secret, a)
    b = rig.sign_variant(host_cli, secret, b)
    run.variant_a, run.variant_b = a, b
    serve_root = workdir / "serve"
    rig.write_serve_root(serve_root, [a, b], base_url)
    rig.populate_cache(nix, secret, serve_root / "cache", [a, b])
    run.bumped_path = rig.package_store_path(b.index, _BUMPED_PACKAGE)
    require(
        run.bumped_path is not None,
        f"index B lists no {console.lit(_BUMPED_PACKAGE)} — nothing would prove the upgrade",
    )
    narinfos = sorted((serve_root / "cache").glob("*.narinfo"))
    require(bool(narinfos), "the rig cache holds no narinfo — nix copy produced nothing")
    run.rig = E2eRig(
        base_url=base_url,
        feed_url=f"{base_url}/{rig.FEED_NAME}",
        cache_url=f"{base_url}/cache",
        cache_public_key=public,
        preflight_urls=[
            f"{base_url}/{rig.FEED_NAME}",
            f"{base_url}/tarballs/{a.tarball.name}",
            f"{base_url}/index/{a.bos_version}/{rig.INDEX_NAME}",
            f"{base_url}/index/{b.bos_version}/{rig.INDEX_NAME}",
            f"{base_url}/cache/{narinfos[0].name}",
        ],
        serve_root=serve_root,
        cache=serve_root / "cache",
        secret=secret,
        host_cli=host_cli,
    )
    return f"{console.lit(base_url)} ({console.lit(len(narinfos))} narinfo(s))"


@stage("Register rig on device")
def register_rig(dev: Device, run: E2eRun) -> str:
    """Point the device at the rig, in two steps: a complete runtime
    servers.json whose factory entry is the rig (the only way to redirect
    init), then register-server for the feed-linked entry + substituter.
    Sysupgrade does not preserve the runtime registry, so this re-runs
    before every flash. The ids must differ — registering under the
    factory's own id is rejected as a collision."""

    r = run.rig
    if r is None:
        msg = "BUG: the rig was not assembled before registration"
        raise RuntimeError(msg)
    config = json.dumps(
        {
            "factory": {
                "id": "e2e-factory",
                "base_url": r.base_url,
                "known_public_key": r.cache_public_key,
                "priority": 0,
                "enabled": True,
            },
            "servers": [],
        }
    )
    dev.run(f"mkdir -p /etc/nix-upgrade && printf '%s' {shlex.quote(config)} > {_SERVERS_JSON}")
    dev.run(
        f"{shlex.quote(_REMOTE_CLI)} register-server --id e2e "
        f"--feed-url {shlex.quote(r.feed_url)} "
        f"--index-public-key {shlex.quote(r.cache_public_key)} "
        f"--cache-url {shlex.quote(r.cache_url)} "
        f"--cache-public-key {shlex.quote(r.cache_public_key)}"
    )
    return (
        f"factory {console.lit('e2e-factory')} + server {console.lit('e2e')}"
        f" → {console.lit(r.base_url)}"
    )


@stage("Clean up server registry")
def cleanup_server_registry(dev: Device) -> str:
    """Remove the runtime servers.json pointing at the ephemeral rig: left
    behind by an aborted run it would win over the shipped defaults and
    send the next real init or upgrade to a dead URL. A completed flash
    drops it anyway — sysupgrade does not preserve the runtime registry."""

    dev.run(f"rm -f {_SERVERS_JSON}")
    return console.lit(_SERVERS_JSON)


@stage("Preflight rig from device")
def preflight_rig(dev: Device, run: E2eRun) -> str:
    """Probe the first bytes of every rig URL from the device itself
    (busybox wget): host-IP autodetection, routing, firewall, or
    URL-generation faults must fail here, not after the store is gone."""

    r = run.rig
    if r is None:
        msg = "BUG: the rig was not assembled before preflight"
        raise RuntimeError(msg)
    failed = [url for url in r.preflight_urls if _first_bytes(dev, url) == 0]
    require(not failed, "the device cannot fetch: " + ", ".join(console.lit(u) for u in failed))
    return f"{console.lit(len(r.preflight_urls))} URLs reachable"


def _first_bytes(dev: Device, url: str) -> int:
    """Bounded reachability probe: how many of the first bytes busybox wget
    pulls from `url`; 0 on any HTTP or routing failure (wget emits nothing
    on error). Counting bytes instead of `&& echo ok` keeps the probe from
    downloading the whole init tarball just to prove the URL works — dd
    caps the read and wget dies on the closed pipe."""

    count = dev.read(
        f"wget -q -O - {shlex.quote(url)} 2>/dev/null | dd bs=64 count=1 2>/dev/null | wc -c"
    )
    try:
        return int(count)
    except ValueError:
        return 0


@stage("Pin device address")
def pin_device_address(
    dev: Device, run: E2eRun, *, resolve: Callable[[str], str] = socket.gethostbyname
) -> str:
    """Resolve --device to a numeric address for the cleardown/flash window:
    the cleardown stops avahi with the generation's other services, so an
    mDNS name can stop resolving mid-run. After each reboot the harness
    returns to the original name — the reboot may take a new DHCP lease."""

    try:
        run.pinned_host = resolve(dev.host)
    except OSError as e:
        raise Abort(f"cannot resolve {console.lit(dev.host)}: {e}") from None
    return console.lit(run.pinned_host)


_ORCHESTRATOR = "bmc-nix-service-orchestrator"


@stage("Clear nix store")
def clear_nix_store(
    dev: Device,
    *,
    assume_yes: bool = False,
    timeout: int = 60,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> str:
    """Stop everything the active generation runs, prove nothing still
    references /nix, unmount, and delete the backing store — the
    destructive premise of the init-path scenario."""

    mounted = bool(dev.read(f"grep ' {_NIX_STORE} ' /proc/mounts || true"))
    backing = bool(dev.read(f"[ -d {_NIX_BACKING} ] && echo yes || true"))
    done_if(not mounted and not backing)
    require(
        assume_yes
        or dry_run.get()
        or console.confirm(
            f"Delete the nix store on {console.lit(dev.host)}? "
            f"This clears {console.lit(_NIX_BACKING)}."
        ),
        "cleardown declined — pass --yes to skip the prompt",
    )
    _quiesce(dev, timeout=timeout, sleep=sleep, clock=clock)
    if dry_run.get():
        return "cleardown logged (dry-run)"
    dev.run(f"rm -rf {_NIX_BACKING}")
    return f"cleared {console.lit(_NIX_BACKING)}"


def _quiesce(
    dev: Device,
    *,
    timeout: int,
    sleep: Callable[[float], None],
    clock: Callable[[], float],
) -> None:
    """Stop everything the active generation runs, prove nothing still
    references /nix, and peel its bind mounts — shared between the
    cleardown (which then deletes) and the B/C fault surgeries (which
    then corrupt)."""
    generation = dev.read(f"readlink -f {_PROFILE_DIR}/current 2>/dev/null || true")
    _clear_orchestrator(dev, timeout=timeout, sleep=sleep, clock=clock)
    if generation:
        _stop_generation_services(dev, generation)
    if dry_run.get():
        return
    failure = _poll_until(
        lambda: _nix_reference_holders(dev), timeout=timeout, sleep=sleep, clock=clock
    )
    if failure is not None:
        raise Abort(f"the store is still referenced: {failure}")
    dev.run("sync")
    # Boot can stack /nix bind mounts (each enabled copy of the activator
    # runs mount_nix), so peel until the mount table is clear.
    for _ in range(8):
        if not dev.read(f"grep ' {_NIX_STORE} ' /proc/mounts || true"):
            break
        dev.run(f"umount {_NIX_STORE}")
    require(
        not dev.read(f"grep ' {_NIX_STORE} ' /proc/mounts || true"),
        f"{console.lit(_NIX_STORE)} is still mounted after umount",
    )


@stage("Quiesce nix services")
def quiesce_nix(
    dev: Device,
    *,
    timeout: int = 60,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> str:
    """The fault suite's non-destructive half of the cleardown: services
    stopped, /nix references gone, bind mounts peeled — the store data
    itself is left for the scenario to corrupt precisely."""
    _quiesce(dev, timeout=timeout, sleep=sleep, clock=clock)
    if dry_run.get():
        return "quiesce logged (dry-run)"
    return f"services stopped, {console.lit(_NIX_STORE)} unmounted"


def _clear_orchestrator(
    dev: Device,
    *,
    timeout: int,
    sleep: Callable[[float], None],
    clock: Callable[[], float],
) -> None:
    """Delete the transient activation orchestrator's ubus instance and wait
    until it is gone BEFORE stopping services — it has no init.d entry, a
    stale instance can linger even with a broken `current` link, and a
    live one could restart what the cleardown stops. The wait is skipped
    under dry-run: the delete is only logged, so the instance never
    disappears."""

    query = shlex.quote(json.dumps({"name": _ORCHESTRATOR}))
    dev.run(f"ubus call service delete {query} 2>/dev/null || true")
    if dry_run.get():
        return
    lingering = _poll_until(
        lambda: _orchestrator_present(dev, query), timeout=timeout, sleep=sleep, clock=clock
    )
    if lingering is not None:
        raise Abort(lingering)


def _orchestrator_present(dev: Device, query: str) -> str | None:
    listed = dev.read(f"ubus call service list {query} 2>/dev/null || true")
    if _ORCHESTRATOR in listed:
        return f"the {_ORCHESTRATOR} ubus instance did not disappear"
    return None


def _stop_generation_services(dev: Device, generation: str) -> None:
    """Run the generation's K* shutdown links in lexical link order —
    ascending priority under OpenWRT's two-digit convention — then
    stop EVERY generation service — including the K*-linked ones (a
    shutdown handler may leave its process running) and disabled ones,
    which may have been started manually."""

    links = dev.read(f"ls {shlex.quote(generation)}/etc/rc.d/ 2>/dev/null || true").split()
    shutdown = [_rc_name(link) for link in sorted(links) if link.startswith("K")]
    for name in shutdown:
        dev.run(f"/etc/init.d/{shlex.quote(name)} shutdown 2>/dev/null || true")
    services = dev.read(f"ls {shlex.quote(generation)}/etc/init.d/ 2>/dev/null || true").split()
    for name in sorted(set(services)):
        dev.run(f"/etc/init.d/{shlex.quote(name)} stop 2>/dev/null || true")


def _nix_reference_holders(dev: Device) -> str | None:
    """pid:comm of processes holding /nix references via exe, cwd, fd, or
    maps — matching bare `/nix` targets too, not only paths under it.
    None when clean."""

    holders = dev.read(
        "for p in /proc/[0-9]*; do "
        "if ls -l $p/exe $p/cwd $p/fd 2>/dev/null | grep -qE ' /nix(/|$)' "
        "|| grep -qE ' /nix(/|$)' $p/maps 2>/dev/null; "
        'then echo "${p#/proc/}:$(cat $p/comm 2>/dev/null)"; fi; done'
    )
    return holders.replace("\n", ", ") or None


def _rc_name(link: str) -> str:
    """Service name behind an rc.d link: strip the S/K prefix and priority."""
    return link[1:].lstrip("0123456789")


def _poll_until(
    check: Callable[[], str | None],
    *,
    timeout: float,
    sleep: Callable[[float], None],
    clock: Callable[[], float],
) -> str | None:
    """Run `check` until it returns None (settled); on timeout, return its
    last failure description."""

    deadline = clock() + timeout
    while True:
        failure = check()
        if failure is None:
            return None
        if clock() >= deadline:
            return failure
        sleep(2)


@stage("Uploaded image on pinned device")
def require_uploaded(dev: Device, image: Image) -> str:
    """Identity tie for the destructive window: the pinned connection must
    hold the exact bytes the name-addressed upload verified. A resolver
    handing out a different unit — several Decks can announce the same
    mDNS name — fails here, before anything destructive runs."""

    if dry_run.get():
        return "upload check skipped (dry-run)"
    require(
        _remote_sha(dev, image.remote_path) == image.sha256,
        f"{console.lit(dev.host)} does not hold the verified upload "
        f"{console.lit(image.remote_path)} — is the pinned address the right device?",
    )
    return f"{console.lit(image.remote_path)} verified via {console.lit(dev.host)}"


@stage("Trust image signing keys")
def trust_image_keys(dev: Device, image: Image) -> str:
    """Install the image's usign public keys on the running system so the
    flash's signature check accepts a dev-signed image on firmware that
    ships different keys. Deliberately not `sysupgrade -F`: force would
    also wave through a failed platform check, and the Nix store staging
    runs inside it. Extraction is transient tmpfs and runs before anything
    destructive, so a failure leaves the device untouched."""

    rootfs = f"{image.sysupgrade_dir}/rootfs.img"
    dev.run(
        "d=$(mktemp -d) && trap 'rm -rf \"$d\"' EXIT && "
        f'tar -xf {shlex.quote(image.remote_path)} -C "$d" {shlex.quote(rootfs)} && '
        f'unsquashfs -q -n -d "$d/r" "$d/{rootfs}" etc/opkg/keys && '
        'mkdir -p /etc/opkg/keys && cp "$d/r/etc/opkg/keys/"* /etc/opkg/keys/'
    )
    return f"{console.lit(image.path.name)} keys → {console.lit('/etc/opkg/keys')}"


@stage("Flash firmware (e2e)")
def flash_e2e(dev: Device, image: Image, *, assume_yes: bool = False) -> str:
    """Flash unconditionally — deliberately no same-version skip: after the
    destructive cleardown a skip would strand the device storeless. The
    bytes were uploaded and verified before anything destructive ran."""

    require(
        assume_yes
        or dry_run.get()
        or console.confirm(
            f"Flash {console.lit(image.version)} to {console.lit(dev.host)}? "
            "The device will reboot."
        ),
        "flash declined — pass --yes to skip the prompt",
    )
    dev.run(f"sysupgrade {shlex.quote(image.remote_path)}", expect_disconnect=True)
    return f"{console.lit(image.version)} → reboot"


@stage("Device on image A's firmware")
def require_lineage(dev: Device, run: E2eRun) -> str:
    """The upgrade scenario runs against image A's lineage — asserted, not
    assumed."""

    version = dev.version
    if dry_run.get():
        return f"device runs {console.lit(version)} (assertion skipped under dry-run)"
    require(
        version == run.image_a.version,
        f"device runs {console.lit(version)}, scenario B expects image A's "
        f"{console.lit(run.image_a.version)}",
    )
    return console.lit(version)


@stage("Store initialized")
def require_initialized_store(dev: Device) -> str:
    """Scenario B's read-only precondition: an upgrade needs an initialized
    store with a promoted generation BEFORE any mutation (registration,
    marker) touches the device — without one the run must abort having
    changed nothing."""

    backing = bool(dev.read(f"[ -d {_NIX_BACKING} ] && echo yes || true"))
    generation = dev.read(f"readlink -f {_PROFILE_DIR}/current 2>/dev/null || true")
    require(
        backing and bool(generation),
        "the store is not initialized — run scenario A (or plain init) first",
    )
    return console.lit(generation)


@stage("Bumped path absent")
def ensure_bump_absent(dev: Device, run: E2eRun) -> str:
    """The bumped store path must not pre-exist: its post-upgrade presence
    is what proves nix-store realised it from the rig cache."""

    path = run.bumped_path
    if path is None:
        msg = "BUG: the rig was not assembled before the bump-absence check"
        raise RuntimeError(msg)
    require(
        "yes" not in dev.read(f"[ -e {shlex.quote(path)} ] && echo yes || true"),
        f"{console.lit(path)} already exists on the device — the upgrade would prove nothing",
    )
    return f"{console.lit(path)} absent"


@stage("Record generation")
def record_generation(dev: Device, run: E2eRun) -> str:
    """Read the same `current` link verify_upgraded compares against —
    variant B's profile — so the advance check can never pass vacuously
    by comparing two unrelated profiles."""

    b = run.variant_b
    if b is None:
        msg = "BUG: e2e artifacts were not built before recording the generation"
        raise RuntimeError(msg)
    current = shlex.quote(f"{b.profile_path}/current")
    run.generation_before = dev.read(f"readlink -f {current} 2>/dev/null || true")
    require(bool(run.generation_before), "no current generation — the store is not initialized")
    return console.lit(run.generation_before)


@stage("Drop e2e marker")
def drop_e2e_marker(dev: Device) -> str:
    """A file outside store/, var/, and the profile tree; GC ignores it and
    is-initialized only checks the directory. Scenario B's preservation
    discriminator: it must survive an in-place upgrade."""

    dev.run(f"touch {_E2E_MARKER}")
    return console.lit(_E2E_MARKER)


@stage("Clean up e2e marker")
def cleanup_e2e_marker(dev: Device) -> str:
    dev.run(f"rm -f {_E2E_MARKER}")
    return console.lit(_E2E_MARKER)


@stage("Sweep uploaded images")
def sweep_uploaded_images(dev: Device, run: E2eRun) -> str:
    """Remove the firmware tars pushed to /tmp — cleanup after a failure or
    a declined flash; a flashed image is gone with the reboot anyway."""

    paths = [run.image_a.remote_path, run.image_b.remote_path]
    dev.run("rm -f " + " ".join(shlex.quote(p) for p in paths))
    return ", ".join(console.lit(p) for p in paths)


@stage("Verify initialized")
def verify_initialized(
    dev: Device,
    run: E2eRun,
    *,
    timeout: int = 300,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> str:
    """Poll, not one-shot: activation happens at boot via nix-activator
    after SSH already answers. The tarball ships a generation but no
    `current` symlink — the activator's latest-generation fallback
    promoting one is part of the contract under test."""

    a = run.variant_a
    if a is None:
        msg = "BUG: e2e artifacts were not built before verification"
        raise RuntimeError(msg)
    if dry_run.get():
        return "verification skipped (dry-run)"

    def check() -> str | None:
        version = dev.read("cat /etc/bos_version 2>/dev/null || true")
        if version != a.bos_version:
            return f"bos_version is {version or '<unset>'}, want {a.bos_version}"
        if not dev.read(f"[ {_NIX_STORE} -ef {_NIX_BACKING} ] && echo yes || true"):
            return f"{_NIX_STORE} is not backed by {_NIX_BACKING}"
        current = shlex.quote(f"{a.profile_path}/current")
        generation = dev.read(f"readlink -f {current} 2>/dev/null || true")
        if not generation:
            return "the activator's fallback did not promote a current generation"
        return _services_not_running(dev, generation)

    failure = _poll_until(_settling(check), timeout=timeout, sleep=sleep, clock=clock)
    if failure is not None:
        _dump_diagnostics(dev)
        raise Abort(f"init verification failed: {failure}")
    return f"{console.lit(a.bos_version)} initialized and active"


@stage("Verify upgraded")
def verify_upgraded(
    dev: Device,
    run: E2eRun,
    *,
    timeout: int = 300,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> str:
    """Poll for the staged `next` generation's boot-time activation: the
    store upgraded in place (marker intact), `current` advanced, the
    bumped path realised from the rig cache, the next.<version-B> marker
    consumed."""

    b, before, bumped = run.variant_b, run.generation_before, run.bumped_path
    if b is None or before is None or bumped is None:
        msg = "BUG: scenario B prerequisites were not recorded before verification"
        raise RuntimeError(msg)
    if dry_run.get():
        return "verification skipped (dry-run)"

    def check() -> str | None:
        version = dev.read("cat /etc/bos_version 2>/dev/null || true")
        if version != b.bos_version:
            return f"bos_version is {version or '<unset>'}, want {b.bos_version}"
        if not dev.read(f"[ -f {_E2E_MARKER} ] && echo yes || true"):
            return "the e2e marker vanished — the store was wiped, not upgraded"
        current = shlex.quote(f"{b.profile_path}/current")
        generation = dev.read(f"readlink -f {current} 2>/dev/null || true")
        if not generation or generation == before:
            return f"current still resolves to {generation or '<nothing>'}"
        manifest = f"{generation}/manifest"
        hit = dev.read(
            f"grep -q {shlex.quote(bumped)} {shlex.quote(manifest)} 2>/dev/null && echo yes || true"
        )
        if not hit:
            return f"the active manifest does not list the bumped path {bumped}"
        names = dev.read(f"ls {shlex.quote(b.profile_path)} 2>/dev/null || true").split()
        if f"next.{b.bos_version}" in names:
            return f"next.{b.bos_version} is still pending — the activator did not consume it"
        return _services_not_running(dev, generation)

    failure = _poll_until(_settling(check), timeout=timeout, sleep=sleep, clock=clock)
    if failure is not None:
        _dump_diagnostics(dev)
        raise Abort(f"upgrade verification failed: {failure}")
    return f"{console.lit(b.bos_version)} upgraded in place"


def _settling(check: Callable[[], str | None]) -> Callable[[], str | None]:
    """Wrap a verification poll check so a transport failure counts as not
    settled — the polls span the reboot window, where ssh can still flap."""

    def wrapped() -> str | None:
        try:
            return check()
        except subprocess.CalledProcessError:
            return "the device is not answering ssh yet"

    return wrapped


def _services_not_running(dev: Device, generation: str) -> str | None:
    """First S*-enabled generation service whose `status` exits nonzero
    (exit 0 = running — the orchestrator's own convention; generated
    services define no `running` action); None when all run. Disabled
    services are not required to run."""

    links = dev.read(f"ls {shlex.quote(generation)}/etc/rc.d/ 2>/dev/null || true").split()
    for name in sorted({_rc_name(link) for link in links if link.startswith("S")}):
        probe = f"/etc/init.d/{shlex.quote(name)} status >/dev/null 2>&1 && echo ok || true"
        if "ok" not in dev.read(probe):
            return f"service {name} is not running"
    return None


def _dump_diagnostics(dev: Device) -> None:
    """Best-effort post-mortem before aborting a verification — boot-time
    activation is otherwise undebuggable, and an unreachable device must
    not turn the abort into a transport traceback. The activator's own
    lines are grepped separately so a chatty boot cannot push them out of
    the tail window; service state is probed, not just listed."""

    service_state = (
        "for s in /etc/init.d/*; do "
        'printf "%s: " "${s##*/}"; '
        '"$s" status >/dev/null 2>&1 && echo running || echo stopped; done'
    )
    for title, cmd in (
        ("logread", "logread 2>/dev/null | tail -n 120"),
        ("nix-activator", "logread 2>/dev/null | grep nix-activator | tail -n 60"),
        ("profile", f"ls -l {_PROFILE_DIR}/ 2>/dev/null"),
        ("manifest", f"head -c 4096 {_PROFILE_DIR}/current/manifest 2>/dev/null"),
        ("service state", service_state),
        ("mounts", "grep -E ' /(nix|mnt/data) ' /proc/mounts 2>/dev/null"),
    ):
        try:
            console.kv(title, dev.read(f"{cmd} || true"))
        except subprocess.CalledProcessError as e:
            console.kv(title, f"unavailable: {e}")


def _mem_available(dev: Device) -> int:
    """Free RAM in bytes; /tmp is swapless tmpfs, so RAM bounds upload+flash."""

    kb = dev.read("awk '/^MemAvailable:/ {print $2}' /proc/meminfo")
    return int(kb) * 1024


def _remote_sha(dev: Device, remote_path: str) -> str:
    """Hex sha256 of the on-device file; empty when absent, so never a false match."""

    return dev.read(f"sha256sum {shlex.quote(remote_path)} 2>/dev/null | cut -d' ' -f1")


def _wait_reachable(
    dev: Device,
    timeout: float,
    *,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> bool:
    deadline = clock() + timeout
    while True:
        if dev.reachable:
            return True
        if clock() >= deadline:
            return False
        sleep(2)


# ── e2e upgrade cycle ─────────────────────────────────────────────────────────

_UPGRADE_SERVER_ID = "dev-upgrade"
_UPGRADE_SERVER_APP = ".#upgrade-server"
# The device serves gRPC(-web) on the plain web port; grpcurl talks h2c to it.
_GRPC_PORT = 80
_GRPC_PACKAGE = "braiins.bmc.web"

_PACKAGE_PHASE_PREFIX = "PACKAGE_UPGRADE_PHASE_"


@dataclass
class UpgradeCycle:
    """Mutable carrier threaded through the e2e upgrade stages."""

    password: str  # device web password; empty when none is set
    port: int  # binary-cache port served on this machine
    index_port: int  # package-index port served on this machine
    key_dir: Path  # upgrade-server signing keypair location
    host: str | None = None  # this machine's address as the device reaches it
    log_path: Path | None = None  # upgrade-server output, for failure diagnosis
    server: "subprocess.Popen[bytes] | None" = None
    cache_public_key: str | None = None
    cookie: str | None = None
    upgrade_id: str | None = None
    generation_before: int | None = None
    installed_before: set[str] | None = None  # package names in the pre-upgrade manifest
    widget_uid: str | None = None  # uid of the widget under install, for the registry check

    @property
    def index_url(self) -> str:
        return f"http://{self.host}:{self.index_port}"

    @property
    def cache_url(self) -> str:
        return f"http://{self.host}:{self.port}"


@stage("grpcurl present")
def ensure_grpcurl() -> None:
    hint = console.lit("nix shell .#pkgs.grpcurl")
    require(shutil.which("grpcurl") is not None, f"grpcurl is not on PATH — get it via {hint}")


@stage("Snapshot current generation")
def snapshot_profile(dev: Device, cycle: UpgradeCycle) -> str:
    cycle.generation_before = _current_generation(dev)
    cycle.installed_before = set(_read_manifest_packages(dev))
    return (
        f"generation {console.lit(cycle.generation_before)}, "
        f"{console.lit(len(cycle.installed_before))} installed package(s)"
    )


def _upgrade_server_argv(
    *, host: str, port: int, index_port: int, key_dir: Path, built: list[Built]
) -> list[str]:
    """The ``nix run .#upgrade-server`` command for the built package set.

    Widget packages (named ``widget-*``) go in as ``--widget`` so the server
    reads their bundled manifest and attaches the picker metadata the frontend
    add-a-widget menu needs; everything else is a plain ``--package`` entry.
    """
    argv = [
        "nix",
        "run",
        _UPGRADE_SERVER_APP,
        "--",
        "--host",
        host,
        "--port",
        str(port),
        "--index-port",
        str(index_port),
        "--key-dir",
        str(key_dir),
    ]
    for b in built:
        flag = "--widget" if b.name.startswith("widget-") else "--package"
        argv += [flag, f"{b.name}={b.version}={b.store_path}"]
    return argv


@stage("Start upgrade server")
def start_upgrade_server(dev: Device, plan: Deployment, cycle: UpgradeCycle) -> str:
    cycle.host = _local_addr(dev.host)
    cycle.log_path = Path(tempfile.gettempdir()) / "bmc-upgrade-server.log"
    argv = _upgrade_server_argv(
        host=cycle.host,
        port=cycle.port,
        index_port=cycle.index_port,
        key_dir=cycle.key_dir,
        built=plan.built,
    )
    with cycle.log_path.open("wb") as log:
        cycle.server = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT)
    _await_upgrade_server(cycle)
    cycle.cache_public_key = (cycle.key_dir / "public").read_text().strip()
    return f"index {console.lit(cycle.index_url)}, cache {console.lit(cycle.cache_url)}"


def stop_upgrade_server(cycle: UpgradeCycle) -> None:
    """Terminate the background upgrade server, if it is still running.

    Not a stage — called from a ``finally`` so the server never outlives
    the procedure, even when a stage aborts.
    """
    server = cycle.server
    if server is None or server.poll() is not None:
        return
    server.terminate()
    try:
        server.wait(timeout=10)
    except subprocess.TimeoutExpired:
        server.kill()
        server.wait()


@stage("Register server on device")
def register_upgrade_server(dev: Device, cycle: UpgradeCycle) -> str:
    key = cycle.cache_public_key
    if key is None:
        msg = "BUG: the upgrade server was not started before registration"
        raise RuntimeError(msg)
    # The index is unsigned, so the index key mirrors the cache key —
    # matches the register-server command upgrade-server itself prints.
    args = [
        _NIX_CLI,
        "register-server",
        "--id",
        _UPGRADE_SERVER_ID,
        "--index-url",
        f"{cycle.index_url}/nix-package-index.v1.json",
        "--index-public-key",
        key,
        "--cache-url",
        cycle.cache_url,
        "--cache-public-key",
        key,
    ]
    inner = " ".join(shlex.quote(a) for a in args)
    dev.run(f"PATH=/run/current-profile/bin:$PATH {inner}")
    return f"{console.lit(_UPGRADE_SERVER_ID)} → {console.lit(cycle.index_url)}"


@stage("Authenticate")
def grpc_login(dev: Device, cycle: UpgradeCycle) -> str:
    response = _grpcurl(dev, "AuthenticationService/Login", data={"password": cycle.password})
    token = response.get("token")
    require(
        isinstance(token, str) and bool(token),
        "login returned no session token — check --password",
    )
    cycle.cookie = f"session_id={token}"
    return "session established"


@stage("Check for upgrade")
def check_for_upgrade(dev: Device, cycle: UpgradeCycle) -> str:
    response = _grpcurl(dev, "UpgradeService/CheckForUpgrade", cookie=cycle.cookie)
    packages = response.get("packages")
    if not isinstance(packages, dict):
        raise Abort(
            "no package upgrade offered — either the served packages match the installed "
            "profile, or they are unavailable (index fetch failed, or servers.json is missing "
            "or unparsable); inspect the raw response below and the device's bmc log to tell "
            f"which (response: {json.dumps(response)})"
        )
    disruption = response.get("disruption")
    require(
        disruption == "UPGRADE_DISRUPTION_APP_RESTART",
        f"expected an APP_RESTART disruption, got {disruption}",
    )
    upgrade_id = response.get("upgradeId")
    require(
        isinstance(upgrade_id, str) and bool(upgrade_id),
        "CheckForUpgrade offered packages but returned no upgrade id",
    )
    cycle.upgrade_id = upgrade_id
    changes = packages.get("changes") or []
    for change in changes:
        version_from = change.get("versionFrom", "?")
        version_to = change.get("versionTo", "?")
        console.kv(change.get("name", "?"), f"{version_from} → {version_to}")
    size = _grpc_mb(packages.get("downloadSizeBytes"))
    console.kv("download size", "unknown" if size is None else f"{size:.1f} MB")
    names = ", ".join(console.lit(change.get("name", "?")) for change in changes)
    return f"{console.lit(len(changes))} change(s): {names}"


def _installable_widget_names(response: dict[str, Any]) -> list[str]:
    """Package names from a GetInstallableWidgets response."""
    return [w["packageName"] for w in response.get("widgets", [])]


def _installed_package_names(list_packages_json: dict[str, Any]) -> list[str]:
    """Package names from `bmc-nix-cli list-packages --format json`."""
    return [p["name"] for p in list_packages_json.get("packages", [])]


@stage("Remove widget for reinstall")
def remove_package(dev: Device, cycle: UpgradeCycle, widget: str) -> str:
    # Idempotent so the e2e re-runs after a mid-flight abort: a run that removed
    # the widget but died before reinstalling leaves it absent, and remove-packages
    # errors on an absent package.
    raw = dev.read(
        f"PATH=/run/current-profile/bin:$PATH {shlex.quote(_NIX_CLI)} list-packages --format json"
    )
    done_if(widget not in _installed_package_names(json.loads(raw)))
    dev.run(
        f"PATH=/run/current-profile/bin:$PATH {shlex.quote(_NIX_CLI)} "
        f"remove-packages --name {shlex.quote(widget)}"
    )
    return f"removed {widget}"


@stage("List installable widgets")
def list_installable_widgets(dev: Device, cycle: UpgradeCycle, widget: str) -> str:
    response = _grpcurl(dev, "UpgradeService/GetInstallableWidgets", cookie=cycle.cookie)
    names = _installable_widget_names(response)
    if widget not in names:
        raise Abort(f"{widget} not offered as installable; got {names}")
    entry = next(w for w in response["widgets"] if w["packageName"] == widget)
    for key in ("uid", "displayName", "category", "icon"):
        if not entry.get(key):
            raise Abort(f"{widget} missing {key} in discovery: {entry}")
    cycle.widget_uid = entry["uid"]
    return f"{widget} installable (uid {entry['uid']})"


@stage("Check for install")
def check_for_install(dev: Device, cycle: UpgradeCycle, widget: str) -> str:
    response = _grpcurl(
        dev,
        "UpgradeService/CheckForUpgrade",
        data={"installPackages": [widget]},
        cookie=cycle.cookie,
    )
    changes = response.get("packages", {}).get("changes", [])
    added = [c for c in changes if c.get("name") == widget and "versionFrom" not in c]
    if not added:
        raise Abort(f"{widget} not an added change in the plan: {changes}")
    cycle.upgrade_id = response.get("upgradeId")
    if not cycle.upgrade_id:
        raise Abort("check returned no upgradeId")
    return f"{widget} planned as install ({cycle.upgrade_id})"


@stage("Verify widget installed")
def verify_widget_installed(dev: Device, cycle: UpgradeCycle, widget: str) -> str:
    raw = dev.read(
        f"PATH=/run/current-profile/bin:$PATH {shlex.quote(_NIX_CLI)} list-packages --format json"
    )
    installed = _installed_package_names(json.loads(raw))
    if widget not in installed:
        raise Abort(f"{widget} not in list-packages after install: {installed}")

    # list-packages only proves the profile carries the package; confirm the
    # running registry actually exposes the widget by uid, so a broken
    # post-install refresh (e.g. a manifest pointing at a missing binary)
    # fails here instead of passing silently.
    uid = cycle.widget_uid
    if not uid:
        raise Abort("widget uid not captured from GetInstallableWidgets")
    response = _grpcurl(dev, "SceneManagementService/GetAvailableWidgets", cookie=cycle.cookie)
    available = {w.get("uid") for w in response.get("widgets", [])}
    if uid not in available:
        raise Abort(f"{widget} (uid {uid}) not exposed by the registry after install: {available}")
    return f"{widget} present in profile and registry (uid {uid})"


@stage("Run upgrade")
def run_upgrade(dev: Device, cycle: UpgradeCycle) -> str:
    upgrade_id = cycle.upgrade_id
    if upgrade_id is None:
        msg = "BUG: CheckForUpgrade did not run before StartUpgrade"
        raise RuntimeError(msg)
    argv = _grpcurl_argv(
        dev,
        "UpgradeService/StartUpgrade",
        data={"upgradeId": upgrade_id},
        cookie=cycle.cookie,
        # No deadline: a realize/download can legitimately run for minutes and a
        # fixed ceiling would abort it. If the device wedges mid-stream this
        # blocks until Ctrl-C, which is acceptable for an interactive dev tool.
        max_time=None,
    )
    events = _stream_events(argv)
    phases = [
        event["packagePhase"] for event in events if isinstance(event.get("packagePhase"), str)
    ]
    for expected in ("REALIZING", "ACTIVATING"):
        require(
            f"{_PACKAGE_PHASE_PREFIX}{expected}" in phases,
            f"the progress stream never reached {expected}; phases seen: {phases}",
        )
    require(
        any("finished" in event for event in events),
        "the progress stream ended without a finished event",
    )
    steps = [phase.removeprefix(_PACKAGE_PHASE_PREFIX) for phase in phases]
    return " → ".join([*steps, "finished"])


@stage("Profile advanced")
def verify_profile_advanced(dev: Device, plan: Deployment, cycle: UpgradeCycle) -> str:
    before = cycle.generation_before
    installed = cycle.installed_before
    if before is None or installed is None:
        msg = "BUG: the profile was not snapshotted before the upgrade"
        raise RuntimeError(msg)
    after = _current_generation(dev)
    require(after > before, f"current generation is still {after}")
    entries = _read_manifest_packages(dev)
    mismatched = []
    for b in plan.built:
        if b.name not in installed:
            # Served but not installed — index-only packages are not
            # auto-installed, so they are not expected to appear.
            continue
        entry = entries.get(b.name)
        got = entry.get("store_path") if isinstance(entry, dict) else None
        if got != b.store_path:
            mismatched.append(f"{b.name} (want {b.store_path}, got {got or 'absent'})")
    require(
        not mismatched,
        f"installed packages not replaced by their served store paths: {'; '.join(mismatched)}",
    )
    return f"generation {console.lit(before)} → {console.lit(after)}"


def _current_generation(dev: Device) -> int:
    link = dev.read(f"readlink {_PROFILE_DIR}/current")
    number = link.rsplit("/", 1)[-1].removesuffix("-link")
    require(
        link.endswith("-link") and number.isdigit(),
        f"unexpected current generation link: {link or '(missing)'}",
    )
    return int(number)


def _read_manifest_packages(dev: Device) -> dict[str, Any]:
    """The `packages` object of the current generation's manifest."""

    raw = dev.read(f"cat {_PROFILE_DIR}/current/manifest")
    try:
        packages = json.loads(raw)["packages"]
    except (json.JSONDecodeError, KeyError, TypeError) as e:
        raise Abort(f"the current manifest is not a package manifest: {e}") from None
    if not isinstance(packages, dict):
        raise Abort("the current manifest's packages entry is not an object")
    return packages


def _local_addr(remote_host: str) -> str:
    """This machine's address on the route to the device — what the device
    must dial to reach the served cache and index."""

    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.connect((remote_host, 1))
            return sock.getsockname()[0]
    except OSError as e:
        raise Abort(f"cannot determine the local address routing to {remote_host}: {e}") from None


def _await_upgrade_server(cycle: UpgradeCycle, *, timeout: float = 300) -> None:
    """Wait until both the cache and the index answer HTTP; the first run of
    `nix run` may still be realizing the app, hence the generous timeout."""

    deadline = time.monotonic() + timeout
    log_hint = f"see {console.lit(cycle.log_path)}"
    urls = (f"{cycle.cache_url}/nix-cache-info", f"{cycle.index_url}/nix-package-index.v1.json")
    for url in urls:
        while not _http_ok(url):
            server = cycle.server
            require(
                server is not None and server.poll() is None,
                f"upgrade-server exited early — {log_hint}",
            )
            require(
                time.monotonic() < deadline,
                f"upgrade-server did not serve {console.lit(url)} in {timeout:.0f}s — {log_hint}",
            )
            time.sleep(1)


def _http_ok(url: str) -> bool:
    try:
        with urllib.request.urlopen(url, timeout=5):
            return True
    except OSError:
        return False


def _grpcurl(
    dev: Device,
    method: str,
    *,
    data: dict[str, Any] | None = None,
    cookie: str | None = None,
) -> dict[str, Any]:
    """Unary gRPC call via grpcurl; returns the decoded response message."""

    argv = _grpcurl_argv(dev, method, data=data, cookie=cookie, max_time=120)
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, check=True)
    except subprocess.CalledProcessError as e:
        raise Abort(f"{method} failed: {e.stderr.strip()}") from None
    try:
        response = json.loads(proc.stdout or "{}")
    except json.JSONDecodeError as e:
        raise Abort(f"{method} returned non-JSON output: {e}") from None
    if not isinstance(response, dict):
        msg = f"BUG: {method} returned a non-message JSON value"
        raise RuntimeError(msg)
    return response


def _grpcurl_argv(
    dev: Device,
    method: str,
    *,
    data: dict[str, Any] | None,
    cookie: str | None,
    max_time: int | None,
) -> list[str]:
    argv = ["grpcurl", "-plaintext"]
    if max_time is not None:
        argv += ["-max-time", str(max_time)]
    if cookie is not None:
        argv += ["-H", f"cookie: {cookie}"]
    argv += ["-d", json.dumps(data or {})]
    return [*argv, f"{dev.host}:{_GRPC_PORT}", f"{_GRPC_PACKAGE}.{method}"]


def _stream_events(argv: list[str]) -> list[dict[str, Any]]:
    """Run a server-streaming grpcurl call, decoding the back-to-back JSON
    messages as they arrive and echoing phase changes live."""

    events: list[dict[str, Any]] = []
    decoder = json.JSONDecoder()
    with subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as proc:
        stdout = proc.stdout
        if stdout is None:
            msg = "BUG: Popen(stdout=PIPE) produced no stdout"
            raise RuntimeError(msg)
        # Drain stderr on a thread: with this loop blocked on stdout, a
        # chatty stderr would fill its 64 KiB pipe and deadlock the child.
        stderr_pipe = proc.stderr
        stderr_chunks: list[str] = []
        stderr_thread: threading.Thread | None = None
        if stderr_pipe is not None:
            stderr_thread = threading.Thread(
                target=lambda: stderr_chunks.append(stderr_pipe.read()),
                daemon=True,
            )
            stderr_thread.start()
        buffer = ""
        for line in stdout:
            buffer = _drain_events(decoder, buffer + line, events)
        if stderr_thread is not None:
            stderr_thread.join()
        stderr = "".join(stderr_chunks)
    if proc.returncode:
        raise Abort(f"StartUpgrade stream failed: {stderr.strip()}")
    return events


def _drain_events(
    decoder: json.JSONDecoder,
    buffer: str,
    events: list[dict[str, Any]],
) -> str:
    """Decode every complete JSON message in `buffer`; return the remainder."""

    while True:
        stripped = buffer.lstrip()
        if not stripped:
            return ""
        try:
            event, end = decoder.raw_decode(stripped)
        except json.JSONDecodeError:
            return stripped
        if isinstance(event, dict):
            events.append(event)
            _print_event(event)
        buffer = stripped[end:]


def _print_event(event: dict[str, Any]) -> None:
    """Echo phase transitions and download progress live (the device
    throttles progress events, so this stays one line per second)."""

    phase = event.get("packagePhase")
    download = event.get("download")
    if isinstance(phase, str):
        console.kv("phase", phase.removeprefix(_PACKAGE_PHASE_PREFIX))
    elif isinstance(download, dict):
        downloaded = _grpc_mb(download.get("downloadedBytes")) or 0.0
        total = _grpc_mb(download.get("totalBytes"))
        suffix = f" / {total:.1f}" if total is not None else ""
        console.kv("download", f"{downloaded:.1f}{suffix} MB")
    elif "finished" in event:
        console.kv("phase", "finished")


def _grpc_mb(value: Any) -> float | None:
    """grpcurl serializes uint64 as a JSON string; None stays None."""

    if value is None:
        return None
    return int(value) / 1_000_000

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

import base64
import difflib
import enum
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from collections.abc import Callable, Iterator
from contextlib import contextmanager, suppress
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any, NewType, Protocol

from rich.markup import escape

from bmc_tui import console, nix_progress, rig
from bmc_tui.bos_version import BosVersion, VersionName, parse_bos_version
from bmc_tui.device import Device, RemotePath
from bmc_tui.image import Image
from bmc_tui.nix import ABSENT_ATTR, Attr, Built, Nix, Pkg, StorePath
from bmc_tui.stage import Abort, best_effort, done_if, dry_run, ensure, require, stage

_PROFILE_DIR = "/nix/var/nix/gcroots/profiles/bmc"
# Probe and invoke the CLI at the profile we deploy into, not
# via the /run/current-profile symlink — the symlink only flips
# to the bmc profile at boot, so right after a bootstrap
# it can disagree with what we just registered.
_NIX_CLI = f"{_PROFILE_DIR}/current/bin/bmc-nix-cli"

_NIX_CONF = "/etc/nix/nix.conf"
_SERVERS_JSON_DEFAULT = "/etc/nix-upgrade/servers.json.default"
_U32_MAX = 0xFFFF_FFFF

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
# OpenWRT's e2fsprogs ships e2fsck/mkfs.ext4 but NOT debugfs, which the B2
# fault surgery needs. Build a statically linked, cross-built debugfs and push
# it like the CLI; the `.bin` output is the lone out-path holding sbin/debugfs.
_DEBUGFS_ATTR = Attr(".#pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic.e2fsprogs.bin")
# Run-unique device paths for the pushed CLI and tarball: Device.push
# truncates via `cat >`, so a constant name would let concurrent
# harness runs upload, read, and clean up the same file.
_REMOTE_CLI = RemotePath(f"/tmp/bmc-nix-cli.deck-init.{os.getpid()}")
# Same run-uniqueness rationale for the pushed debugfs; /tmp is tmpfs, so a
# reboot sweeps it, and cleanup_remote_artifacts removes it like the CLI.
_REMOTE_DEBUGFS = RemotePath(f"/tmp/bmc-debugfs.deck-faults.{os.getpid()}")

# Sysupgrade stages the tar in /tmp and extracts rootfs.img before pivoting.
FLASH_HEADROOM = 20 * 1024 * 1024


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
    listed = dev.read("ls -1 /tmp/*.tar 2>/dev/null || true")
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


@stage("Compatible bmc-nix-cli present")
def ensure_nix_cli(nix: Nix, dev: Device) -> None:
    def compatible() -> bool:
        cli = shlex.quote(_NIX_CLI)
        return "ok" in dev.read(
            f"test -x {cli} && {cli} register-server --help 2>/dev/null "
            "| grep -q -- --exclusive && echo ok || true"
        )

    def bootstrap() -> None:
        [built] = nix.build([nix.resolve(Attr(".#deck-packages.bmc-nix-cli"))])
        nix.copy([built.store_path], dev.copy_dest)
        dev.run(_register_cmd([built], cli=f"{built.store_path}/bin/bmc-nix-cli"))

    ensure(compatible, bootstrap, "compatible bmc-nix-cli bootstrap did not take")


# The flake sets `lfs = true`, so nix resolves LFS pointers from the remote
# while it evaluates. A committed but unpushed object 404s, and only the attrs
# whose derivations read the file fail — `.version` still evaluates.
_LFS_404 = "lfs/objects/batch"
_OID = re.compile(r'"oid"\s*:\s*"([0-9a-f]{64})"')

_FAILURE_TITLE = "Cannot resolve packages"
_LFS_SUMMARY = "nix could not fetch a git-LFS object this tree references"


def _resolve_failure(attr: str, stderr: str, nix: Nix, prefix: str) -> str:
    """Panel the diagnosis where a single line cannot hold it,
    and return the line to abort on.

    Reporting every failure as "does not exist" sends the reader hunting
    a typo whatever actually broke.
    """
    if _LFS_404 in stderr:
        found = _OID.search(stderr)
        named = f"\n\n  {console.lit(found.group(1))}" if found else ""
        console.panel(
            f"{_LFS_SUMMARY}:{named}\n\n"
            "The flake resolves LFS from the remote as it evaluates, so an object that "
            "is committed but not yet pushed is invisible to it.\n\n"
            f"To fix:  {console.lit('git lfs push origin HEAD')}",
            title=_FAILURE_TITLE,
            style="red",
        )
        return _LFS_SUMMARY
    if ABSENT_ATTR in stderr:
        return _unknown_package_hint(attr, nix.list_packages(prefix), prefix)
    # Escaped: nix's errors carry bracketed spans that rich would read as markup.
    console.panel(escape(_without_warnings(stderr)), title=_FAILURE_TITLE, style="red")
    return f"nix failed to evaluate {console.lit(attr)}"


def _without_warnings(stderr: str) -> str:
    """Drop nix's own `warning:` lines so the error reads as just the error."""
    kept = (line for line in stderr.splitlines() if not line.startswith("warning:"))
    return "\n".join(kept).strip()


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
        except subprocess.CalledProcessError as e:
            raise Abort(_resolve_failure(attr, e.stderr or "", nix, plan.prefix)) from None
    plan.resolved = resolved
    # nix's own notice is switched off on the build calls, so say it here, once.
    if nix.dirty_tree():
        console.warn("worktree has uncommitted changes — this build is not in git")
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


@stage("Clear upgrade servers")
def clear_upgrade_servers(dev: Device) -> str:
    try:
        dev.run(f"{_NIX_CLI} clear-servers")
    except subprocess.CalledProcessError as error:
        # A plan without bmc-nix-cli leaves the previously deployed CLI in
        # the profile, and that one may predate the subcommand.
        if "unrecognized subcommand" not in (error.stderr or ""):
            raise
        return "on-device bmc-nix-cli predates clear-servers; servers left untouched"
    if dry_run.get():
        return "would clear package servers"
    return "package servers cleared; scheduled upgrade checks fail until servers are re-registered"


def _register_server_cmd(entry: dict[str, object]) -> str:
    parts = [_NIX_CLI, "register-server", "--id", str(entry["id"])]
    if entry.get("feed_url") is not None:
        parts += ["--feed-url", str(entry["feed_url"])]
    else:
        parts += ["--index-url", str(entry["index_url"])]
    parts += ["--index-public-key", str(entry["known_public_key"])]
    parts += ["--priority", str(entry.get("priority", 50))]
    if not entry.get("required", True):
        parts.append("--optional")
    return shlex.join(parts)


def _default_server_entries(raw: str) -> list[dict[str, object]]:
    if not raw.strip():
        return []
    try:
        registry = json.loads(raw)
    except json.JSONDecodeError as error:
        raise Abort(f"{_SERVERS_JSON_DEFAULT} is not valid JSON: {error}") from None
    require(
        isinstance(registry, dict),
        f"{_SERVERS_JSON_DEFAULT} must contain a JSON object",
    )
    servers = registry.get("servers", [])
    require(
        isinstance(servers, list),
        f"{_SERVERS_JSON_DEFAULT} must contain a servers list",
    )

    entries: list[dict[str, object]] = []
    for index, candidate in enumerate(servers):
        label = f"{_SERVERS_JSON_DEFAULT} servers[{index}]"
        require(isinstance(candidate, dict), f"{label} must be a JSON object")

        entry_id = candidate.get("id")
        require(
            isinstance(entry_id, str) and bool(entry_id),
            f"{label}.id must be a non-empty string",
        )
        sources = [
            source for source in ("feed_url", "index_url") if candidate.get(source) is not None
        ]
        require(
            len(sources) == 1,
            f"{label} must contain exactly one of feed_url or index_url",
        )
        source = sources[0]
        require(
            isinstance(candidate[source], str) and bool(candidate[source]),
            f"{label}.{source} must be a non-empty string",
        )
        known_public_key = candidate.get("known_public_key")
        require(
            isinstance(known_public_key, str) and bool(known_public_key),
            f"{label}.known_public_key must be a non-empty string",
        )
        priority = candidate.get("priority")
        require(
            isinstance(priority, int)
            and not isinstance(priority, bool)
            and 0 <= priority <= _U32_MAX,
            f"{label}.priority must be an unsigned 32-bit integer",
        )
        require(
            isinstance(candidate.get("enabled"), bool),
            f"{label}.enabled must be a boolean",
        )
        require(
            isinstance(candidate.get("required", True), bool),
            f"{label}.required must be a boolean when present",
        )
        entries.append(candidate)
    return entries


@stage("Register upgrade servers")
def register_default_servers(
    dev: Device,
    url: str | None = None,
    entry_id: str | None = None,
    key: str | None = None,
) -> str:
    # Registry reads must run under --dry-run so the skipped mutations can be derived and logged.
    raw = dev.read(f"cat {_SERVERS_JSON_DEFAULT} 2>/dev/null || true")

    entries = _default_server_entries(raw)
    # register-server always writes enabled entries, so replaying a disabled
    # one would silently enable it.
    disabled = [entry for entry in entries if entry["enabled"] is False]
    entries = [entry for entry in entries if entry["enabled"] is True]

    if not entries:
        require(
            url is not None and entry_id is not None and key is not None,
            f"{_SERVERS_JSON_DEFAULT} has no enabled server entries; "
            "pass --url, --id and --key to register one explicitly",
        )
        explicit_entry: dict[str, object] = {
            "id": entry_id,
            "index_url": url,
            "known_public_key": key,
            "priority": 50,
        }
        entries = [explicit_entry]
    elif url is not None or entry_id is not None or key is not None:
        # Without this, --id naming a disabled entry falls through to the
        # rename below and registers a different server's URL and key under
        # the requested id.
        if entry_id is not None:
            require(
                all(entry["id"] != entry_id for entry in disabled),
                f"default entry {entry_id!r} is disabled; enable it in "
                f"{_SERVERS_JSON_DEFAULT} before replaying it",
            )
        if len(entries) > 1 and entry_id is not None:
            entries = [entry for entry in entries if entry["id"] == entry_id]
            require(bool(entries), f"no enabled default entry with id {entry_id!r}")
        require(
            len(entries) == 1,
            "overrides are ambiguous with multiple default entries; pass --id",
        )
        entry = dict(entries[0])
        if entry_id is not None:
            entry["id"] = entry_id
        if key is not None:
            entry["known_public_key"] = key
        if url is not None:
            source = "feed_url" if entry.get("feed_url") is not None else "index_url"
            entry[source] = url
        entries = [entry]

    commands = [_register_server_cmd(entry) for entry in entries]
    for command in commands:
        dev.run(command)
    ids = ", ".join(str(entry["id"]) for entry in entries)
    if disabled:
        skipped = ", ".join(str(entry["id"]) for entry in disabled)
        return f"registered {ids}; skipped disabled {skipped}"
    return f"registered {ids}"


_WASM_HOST = "bmc-wasm-host"
_WIDGET_RELOAD_HOOK = "/run/current-profile/core/activation/scripts/999-signal-widget-reload"


@stage("Stop compositor")
def stop_compositor(
    dev: Device,
    *,
    timeout: int = 60,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> str:
    """Stop the service so its widget teardown releases bmc-wasm-host.

    The stop itself is synchronous, but the wasm hosts exit on their own
    after the widget teardown — wait them out, or the memory gate that
    follows samples RAM they still hold.
    """
    dev.run("service bmc-compositor stop")
    if dry_run.get():
        return "stop logged (dry-run)"
    failure = _poll_until(
        lambda: dev.read(f"pidof {_WASM_HOST} || true").strip() or None,
        timeout=timeout,
        sleep=sleep,
        clock=clock,
    )
    require(failure is None, f"bmc-wasm-host still running after stop: pid(s) {failure}")
    return "stopped"


Pid = NewType("Pid", str)


def compositor_pid(dev: Device) -> Pid | None:
    """PID of the running compositor, None when it is down."""
    found = dev.read(f"pidof {_COMPOSITOR} | cut -d' ' -f1").strip()
    return Pid(found) if found else None


def start_compositor(dev: Device) -> None:
    """Cleanup counterpart of stop_compositor; a no-op when already up."""
    dev.run("service bmc-compositor start")


def _await_orchestrator(dev: Device, timeout: int = _ORCHESTRATOR_TIMEOUT) -> None:
    """Block while the post-activation service reconciliation is still running.

    It is spawned detached and waits on the profile lock.
    That lets it bounce the compositor after `add-packages` has returned,
    so sampling before it settles would miss the restart it is about to make.
    """
    lingering = dev.read(
        f"i=0; while pidof {_ORCHESTRATOR} >/dev/null 2>&1 && [ $i -lt {timeout} ]; "
        f"do sleep 1; i=$((i+1)); done; pidof {_ORCHESTRATOR} || true"
    )
    require(
        not lingering,
        f"service orchestrator still reconciling after {timeout} s (pid(s) {lingering})",
    )


@stage("Restart compositor")
def restart_compositor(dev: Device) -> str:
    """Hard-restart the compositor for diagnostic procedure cleanup."""
    if dry_run.get():
        return "skipped (dry-run)"

    dev.run("/etc/init.d/bmc-compositor restart")
    return "restarted"


@stage("Wait for package activation")
def await_package_activation(dev: Device, *, old_pid: Pid | None) -> str:
    """Wait for activation to reload widgets or reconcile the compositor service."""
    if dry_run.get():
        return "skipped (dry-run)"

    _await_orchestrator(dev)
    now = compositor_pid(dev)
    if now is None:
        raise Abort("compositor is not running after package activation")
    if old_pid is None:
        return f"compositor started during activation (pid {now})"
    if now != old_pid:
        return f"compositor restarted by the service orchestrator (pid {now})"
    if dev.read(f"test -x {_WIDGET_RELOAD_HOOK} && echo yes || true") == "yes":
        return "activation completed; compositor undisturbed (widget reload hook present)"

    raise Abort("active core cannot reload widgets; deploy a current core package first")


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
    debugfs: Path | None = None  # host path of the built static debugfs (B2 only)
    servers_snapshot: "FileSnapshot | None" = None
    nix_conf_snapshot: "FileSnapshot | None" = None


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


@stage("Build debugfs")
def build_debugfs(nix: Nix, plan: Provisioning) -> str:
    """Cross-build the static ARM debugfs the B2 surgery needs; lazy — only
    scenarios that corrupt ext4 metadata call this, never every invocation."""

    out = Path(nix.build_out(_DEBUGFS_ATTR))
    debugfs = out / "sbin/debugfs"
    require(debugfs.is_file(), f"no sbin/debugfs in {console.lit(out)}")
    plan.debugfs = debugfs
    return console.lit(debugfs.name)


@stage("Push debugfs")
def push_debugfs(dev: Device, plan: Provisioning) -> str:
    """Push the cross-built debugfs to a tmpfs /tmp path (a reboot sweeps it;
    cleanup_remote_artifacts removes it explicitly, like the CLI)."""

    debugfs = plan.debugfs
    if debugfs is None:
        msg = "BUG: debugfs was not built before the push stage"
        raise RuntimeError(msg)
    dev.push(debugfs, _REMOTE_DEBUGFS)
    dev.run(f"chmod +x {shlex.quote(_REMOTE_DEBUGFS)}")
    return f"→ {console.lit(_REMOTE_DEBUGFS)}"


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
    """Remove the pushed CLI, tarball, and B2 debugfs; runs also after a
    failed stage."""

    paths = [p for p in (plan.remote_tarball, _REMOTE_CLI, _REMOTE_DEBUGFS) if p]
    dev.run(f"rm -f {' '.join(shlex.quote(p) for p in paths)}")
    return ", ".join(console.lit(p) for p in paths)


# ── sysupgrade e2e ────────────────────────────────────────────────────────────

_E2E_ARTIFACTS_FILE = "nix/e2e-artifacts.nix"
_E2E_ATTRS = ["index-a", "tarball-a", "index-b", "tarball-b"]
SERVERS_JSON = "/etc/nix-upgrade/servers.json"
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
        require_nix_era(image)
    return f"A {console.lit(a)}, B {console.lit(b)}"


@stage("Validate firmware image (nix-era)")
def require_nix_era(image: Image) -> None:
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
    dev.run(f"mkdir -p /etc/nix-upgrade && printf '%s' {shlex.quote(config)} > {SERVERS_JSON}")
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


def _remote_file_b64(dev: Device, path: str) -> str | None:
    """base64 of a remote file's bytes, or None when it is absent."""
    out = dev.read(f"if [ -e {path} ]; then echo PRESENT; base64 {path}; else echo ABSENT; fi")
    marker, _, body = out.partition("\n")
    require(marker in ("PRESENT", "ABSENT"), f"unexpected capture output: {out[:80]!r}")
    return "".join(body.split()) if marker == "PRESENT" else None


@stage("Capture server registry")
def capture_server_registry(dev: Device, plan: "Provisioning | UpgradeCycle") -> str:
    """Record the pre-run runtime servers.json so the final restore puts
    the device back exactly as found. The run registers the ephemeral rig
    there; leaving that behind would send the next real init or upgrade
    to a dead URL, but deleting the file outright would wipe a real
    registration that sysupgrade deliberately preserves (it is a
    registered conffile)."""

    plan.servers_snapshot = snapshot_remote_file(dev, SERVERS_JSON)
    state = "captured" if plan.servers_snapshot.present else "absent before the run"
    return f"{console.lit(SERVERS_JSON)} {state}"


@stage("Restore server registry")
def restore_server_registry(dev: Device, plan: "Provisioning | UpgradeCycle") -> str:
    """Put the runtime servers.json back exactly as capture_server_registry
    found it: restore the original bytes, or remove the file if there was
    none — so the rig registration never outlives the run and a real
    pre-run registration survives it."""

    snapshot = plan.servers_snapshot
    if snapshot is None:
        raise Abort("BUG: restore without a prior capture")
    restore_remote_file(dev, snapshot)
    if not snapshot.present:
        return f"{console.lit(SERVERS_JSON)} removed (absent before the run)"
    return f"{console.lit(SERVERS_JSON)} restored"


@stage("Capture nix.conf")
def capture_nix_conf(dev: Device, plan: "Provisioning | UpgradeCycle") -> str:
    """Record nix.conf before registration writes to it.

    `register-server` adds the rig's `extra-substituters` and, worse, an
    `extra-trusted-public-keys` entry: a standing grant for a developer
    machine's signing key on a device that will outlive the run."""

    plan.nix_conf_snapshot = snapshot_remote_file(dev, _NIX_CONF)
    state = "captured" if plan.nix_conf_snapshot.present else "absent before the run"
    return f"{console.lit(_NIX_CONF)} {state}"


@stage("Restore nix.conf")
def restore_nix_conf(dev: Device, plan: "Provisioning | UpgradeCycle") -> str:
    snapshot = plan.nix_conf_snapshot
    if snapshot is None:
        raise Abort("BUG: restore without a prior capture")
    restore_remote_file(dev, snapshot)
    if not snapshot.present:
        return f"{console.lit(_NIX_CONF)} removed (absent before the run)"
    return f"{console.lit(_NIX_CONF)} restored"


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


class _Pinnable(Protocol):
    pinned_host: str | None


@stage("Pin device address")
def pin_device_address(
    dev: Device, run: _Pinnable, *, resolve: Callable[[str], str] = socket.gethostbyname
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


# ── data-partition fault surgery ──────────────────────────────────────────────

_DATA_MOUNT = "/mnt/data"


@dataclass
class DataPartition:
    """The released block device's identity, recorded before corruption."""

    device: str
    majmin: str
    uuid: str


@dataclass
class UpgradeState:
    """The untouched-state contract the C-group asserts around an abort."""

    current: str
    marker_present: bool
    next_markers: list[str]
    boot_id: str


@dataclass
class FaultsState:
    """Mutable cross-stage values of one fault scenario — the stage engine
    returns None, so value-producing stages write here (mirrors E2eRun)."""

    partition: DataPartition | None = None
    upgrade_state: UpgradeState | None = None
    flash_output: str | None = None
    servers_json_before: str | None = None  # base64 of the pre-D5-flash registry


# A mountinfo line needs field 3 (major:minor), field 5 (mount point), and the
# two fields after the optional-fields '-' separator, so it is only usable with
# at least this many whitespace-split fields.
_MOUNTINFO_MIN_FIELDS = 5


def _mountinfo_entries(dev: Device) -> list[tuple[str, str, str]]:
    """(major:minor, mount point, source) triples from /proc/self/mountinfo.
    The source is the second field after the optional-fields '-' separator;
    matching by major:minor (field 3) is immune to path aliases and bind
    mounts — the same identity partition.rs compares."""
    entries = []
    for line in dev.read("cat /proc/self/mountinfo").splitlines():
        fields = line.split()
        if "-" not in fields or len(fields) < _MOUNTINFO_MIN_FIELDS:
            continue
        separator = fields.index("-")
        entries.append((fields[2], fields[4], fields[separator + 2]))
    return entries


def _blkid_uuid(dev: Device, device: str) -> str:
    out = dev.read(f"blkid {shlex.quote(device)} || true")
    match = re.search(r'UUID="([^"]+)"', out)
    require(match is not None, f"blkid reports no UUID for {console.lit(device)}: {out!r}")
    assert match is not None  # narrowed by require
    return match.group(1)


@stage("Release data partition")
def release_data_partition(dev: Device, state: FaultsState) -> str:
    """Resolve the data partition and its major:minor while still mounted,
    record its ext4 UUID, unmount, then prove from a mountinfo re-read
    that no remaining mount is backed by that device — raw writes on a
    mounted filesystem are never permitted."""
    entry = next((e for e in _mountinfo_entries(dev) if e[1] == _DATA_MOUNT), None)
    require(entry is not None, f"{console.lit(_DATA_MOUNT)} is not mounted — nothing to release")
    assert entry is not None  # narrowed by require
    majmin, _mount, device = entry
    uuid = _blkid_uuid(dev, device)
    dev.run(f"umount {shlex.quote(_DATA_MOUNT)}")
    state.partition = DataPartition(device=device, majmin=majmin, uuid=uuid)
    if dry_run.get():
        return "release logged (dry-run)"
    still = [e for e in _mountinfo_entries(dev) if e[0] == majmin]
    require(
        not still,
        f"device {console.lit(device)} ({majmin}) still backs mounts: "
        + ", ".join(m for _, m, _src in still),
    )
    return f"{console.lit(device)} ({majmin}) released, UUID {console.lit(uuid)}"


def _require_partition(state: FaultsState) -> DataPartition:
    if state.partition is None:
        msg = "BUG: the data partition was not released before corruption"
        raise RuntimeError(msg)
    return state.partition


@stage("Blank ext4 signature (B1)")
def corrupt_partition_blank(dev: Device, state: FaultsState) -> str:
    """Zero the superblock region so blkid sees no filesystem — the ladder
    must take the mkfs branch (observable: a new UUID)."""
    p = _require_partition(state)
    dev.run(f"dd if=/dev/zero of={shlex.quote(p.device)} bs=64k count=1 conv=fsync")
    return f"first 64 KiB of {console.lit(p.device)} zeroed"


# The pinned B2 recipe, single-sourced: the loopback fixture test proves
# these exact commands drive the fsck ladder's repair branch.
_B2_DEBUGFS_COMMANDS: tuple[str, str] = ("sif <2> links_count 0", "ssv state 2")


@stage("Corrupt ext4 metadata (B2)")
def corrupt_partition_metadata(
    dev: Device, state: FaultsState, *, debugfs: str = _REMOTE_DEBUGFS
) -> str:
    """The pinned repair-branch recipe: zero the root inode's link count and
    set the superblock errors flag. The flag makes plain `e2fsck -p` run
    its full check (exit 4); `e2fsck -y` repairs (exit 1); the UUID must
    survive (repaired, never reformatted). Validated on e2fsprogs 1.47.3
    and re-proven by the loopback fixture test.

    `debugfs` is the pushed static binary's device path — OpenWRT's
    e2fsprogs has no debugfs, so the harness ships its own (push_debugfs)."""
    p = _require_partition(state)
    for command in _B2_DEBUGFS_COMMANDS:
        dev.run(f"{shlex.quote(debugfs)} -w -R '{command}' {shlex.quote(p.device)}")
    return f"root links_count zeroed + errors flag set on {console.lit(p.device)}"


@stage("Filesystem identity changed (mkfs proof)")
def require_fs_uuid_changed(dev: Device, state: FaultsState) -> str:
    if dry_run.get():
        return "UUID probe skipped (dry-run: mkfs was only logged, the UUID cannot have changed)"
    p = _require_partition(state)
    uuid = _blkid_uuid(dev, p.device)
    require(uuid != p.uuid, f"UUID still {console.lit(uuid)} — mkfs did not run")
    return f"{console.lit(p.uuid)} → {console.lit(uuid)}"


@stage("Filesystem identity unchanged (repair proof)")
def require_fs_uuid_unchanged(dev: Device, state: FaultsState) -> str:
    if dry_run.get():
        return "UUID probe skipped (dry-run: the corruption and repair were only logged)"
    p = _require_partition(state)
    uuid = _blkid_uuid(dev, p.device)
    require(uuid == p.uuid, f"UUID changed to {console.lit(uuid)} — the ladder reformatted")
    return f"UUID {console.lit(uuid)} preserved"


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
def flash_e2e(
    dev: Device,
    image: Image,
    *,
    assume_yes: bool = False,
    remote_path: str | None = None,
    state: FaultsState | None = None,
) -> str:
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
    command = f"sysupgrade {shlex.quote(remote_path or image.remote_path)}"
    if state is None:
        dev.run(command, expect_disconnect=True)
        return f"{console.lit(image.version)} → reboot"
    outcome = dev.run_captured(command)
    if outcome is not None:
        require(
            outcome.status != "failed",
            f"sysupgrade failed (exit {outcome.returncode}): {outcome.output[-2000:]}",
        )
        state.flash_output = outcome.output
    return f"{console.lit(image.version)} → reboot (output captured)"


# The lines COMMAND's v() prints when it actually stages (init_nix_store /
# stage_nix_upgrade); the dedupe-marker skip prints "already prepared"
# instead. sysupgrade exports VERBOSE=1, so ssh captures them. D4 counts
# the union: exactly one across the whole flash output.
_STAGING_TOKENS = ("Initializing Nix store", "Staging Nix profile for the new firmware")

# bmc-nix downloads the factory tarball to the FIXED path
# <download-dir>/init-tarball.tar.gz (store.rs joins that constant name onto
# --download-dir, which COMMAND sets to /mnt/data); the served feed filename
# never appears on the device.
_DOWNLOAD_ARTIFACT = f"{_DATA_MOUNT}/init-tarball.tar.gz"


def _next_markers(dev: Device) -> list[str]:
    # An unescaped dot keeps the literal "next." in the emitted command (what
    # the harness tests match on); the filenames are always next.<version>,
    # so treating the dot as any-char over-matches nothing that exists.
    listed = dev.read(f"ls {_PROFILE_DIR}/ 2>/dev/null | grep '^next.' || true")
    return listed.split()


@stage("Flash expecting abort")
def flash_expect_abort(  # noqa: PLR0913  the abort contract needs image, expect, and state together
    dev: Device,
    image: Image,
    *,
    expect: str | tuple[str, ...],
    state: FaultsState,
    assume_yes: bool = False,
    remote_path: str | None = None,
) -> str:
    """The fault must abort sysupgrade before flashing: require a remote
    nonzero exit (session death here means the fault likely failed to
    prevent the flash), a per-scenario message in the captured output,
    and an unchanged firmware version afterwards."""
    require(
        assume_yes
        or dry_run.get()
        or console.confirm(
            f"Attempt a flash of {console.lit(image.version)} on {console.lit(dev.host)} "
            "expecting it to abort?"
        ),
        "flash declined — pass --yes to skip the prompt",
    )
    patterns = (expect,) if isinstance(expect, str) else expect
    version_before = dev.version
    outcome = dev.run_captured(f"sysupgrade {shlex.quote(remote_path or image.remote_path)}")
    if outcome is None:
        return "abort expected (dry-run)"
    state.flash_output = outcome.output
    tail = outcome.output[-2000:]
    require(
        outcome.status != "session-death",
        f"session lost during an expect-abort flash — the fault may not have "
        f"prevented the flash (exit {outcome.returncode}): {tail}",
    )
    require(
        outcome.status == "failed",
        f"sysupgrade exited cleanly — the fault did not fire: {tail}",
    )
    matched = next((p for p in patterns if p in outcome.output), None)
    require(
        matched is not None,
        f"abort output does not mention any of {patterns!r}: {tail}",
    )
    version_after = dev.version
    require(
        version_after == version_before,
        f"firmware changed {version_before} → {version_after} despite the abort",
    )
    return f"aborted with {console.lit(matched)}, still on {console.lit(version_before)}"


@stage("Store absent")
def require_store_absent(dev: Device) -> str:
    require(
        not dev.read(f"[ -d {_NIX_BACKING} ] && echo yes || true"),
        f"{console.lit(_NIX_BACKING)} exists — the store is not absent",
    )
    return f"{console.lit(_NIX_BACKING)} absent"


_BOOT_ID = "/proc/sys/kernel/random/boot_id"


@stage("Record upgrade state")
def record_upgrade_state(dev: Device, state: FaultsState) -> str:
    state.upgrade_state = UpgradeState(
        current=dev.read(f"readlink -f {_PROFILE_DIR}/current 2>/dev/null || true"),
        marker_present=bool(dev.read(f"[ -f {_E2E_MARKER} ] && echo yes || true")),
        next_markers=_next_markers(dev),
        boot_id=dev.read(f"cat {_BOOT_ID}"),
    )
    return f"current {console.lit(state.upgrade_state.current)}"


@stage("Reboot happened")
def require_rebooted(dev: Device, state: FaultsState) -> str:
    """A same-version re-flash proves nothing through the version check,
    and the untouched-state contract is satisfied by doing nothing at all:
    a sysupgrade that exits zero without flashing or rebooting would pass
    C6/D1/D4 green. The kernel boot id is fresh per boot — requiring it to
    change proves the flash really went through a reboot."""
    before = state.upgrade_state
    if before is None:
        msg = "BUG: upgrade state was not recorded before the flash"
        raise RuntimeError(msg)
    if dry_run.get():
        return "reboot probe skipped (dry-run: the flash was only logged)"
    boot_id = dev.read(f"cat {_BOOT_ID}")
    require(
        boot_id != before.boot_id,
        f"boot id still {console.lit(boot_id)} — sysupgrade exited without rebooting",
    )
    return f"boot id {console.lit(before.boot_id)} → {console.lit(boot_id)}"


@stage("Upgrade state untouched")
def require_upgrade_state_untouched(dev: Device, state: FaultsState) -> str:
    before = state.upgrade_state
    if before is None:
        msg = "BUG: upgrade state was not recorded before the abort attempt"
        raise RuntimeError(msg)
    current = dev.read(f"readlink -f {_PROFILE_DIR}/current 2>/dev/null || true")
    require(current == before.current, f"current moved: {before.current} → {current}")
    marker = bool(dev.read(f"[ -f {_E2E_MARKER} ] && echo yes || true"))
    require(marker == before.marker_present, "the e2e marker changed across the abort")
    markers = _next_markers(dev)
    require(
        markers == before.next_markers,
        f"next.* markers changed: {before.next_markers} → {markers}",
    )
    return "store, marker, current, and next.* untouched"


@stage("Sweep download artifact")
def sweep_download_artifact(dev: Device) -> str:
    """Init downloads the factory tarball to the FIXED path
    <download-dir>/init-tarball.tar.gz (store.rs joins that constant
    name onto --download-dir, which COMMAND sets to /mnt/data); the
    served feed filename never appears on the device. Stall and
    mid-download failures leave the partial file behind — the harness
    sweeps it here."""
    dev.run(f"rm -f {shlex.quote(_DOWNLOAD_ARTIFACT)}")
    if dry_run.get():
        return f"sweep of {console.lit(_DOWNLOAD_ARTIFACT)} logged (dry-run)"
    require(
        not dev.read(f"[ -e {shlex.quote(_DOWNLOAD_ARTIFACT)} ] && echo yes || true"),
        f"{console.lit(_DOWNLOAD_ARTIFACT)} survived the sweep",
    )
    return f"{console.lit(_DOWNLOAD_ARTIFACT)} absent"


@stage("Download artifact absent")
def require_download_artifact_absent(dev: Device) -> str:
    require(
        not dev.read(f"[ -e {shlex.quote(_DOWNLOAD_ARTIFACT)} ] && echo yes || true"),
        f"{console.lit(_DOWNLOAD_ARTIFACT)} exists — bytes were fetched or left behind",
    )
    return f"{console.lit(_DOWNLOAD_ARTIFACT)} absent"


@stage("Staging ran once (D4)")
def require_staged_once(state: FaultsState) -> str:
    if dry_run.get():
        # the flash was logged-and-skipped, so no output was captured — match
        # verify_initialized/verify_upgraded rather than raise the BUG guard
        return "staged-once check skipped (dry-run)"
    output = state.flash_output
    if output is None:
        msg = "BUG: no flash output was captured for the staged-once check"
        raise RuntimeError(msg)
    count = sum(output.count(token) for token in _STAGING_TOKENS)
    require(
        count == 1,
        f"nix staging lines appeared {count} times, expected exactly once "
        "(the double-validation dedupe failed, or sysupgrade ran without "
        "VERBOSE=1 and the lines never reached the captured output)",
    )
    return "staging line seen once"


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


# ── tampered registration, store surgery, /dev/shm staging ────────────────────

_WITNESS = f"{_NIX_BACKING}/.bdk601-witness"
_SHM_DIR = "/dev/shm"
_STALE_NEXT_MARKER = "next.9999-99-99-0-deadbeef"


def shm_path(image: Image) -> RemotePath:
    return RemotePath(f"{_SHM_DIR}/{image.path.name}")


@stage("Register rig with wrong cache key (C4)")
def register_rig_tampered(dev: Device, run: E2eRun, *, wrong_public_key: str) -> str:
    """register_rig with every key argument replaced by a same-name wrong
    key: registration replaces a nix.conf key only on a name match, so a
    differently-named key would leave the good key trusted and the fault
    would not fire. The good key must be gone afterwards."""
    r = run.rig
    if r is None:
        msg = "BUG: the rig was not assembled before registration"
        raise RuntimeError(msg)
    config = json.dumps(
        {
            "factory": {
                "id": "e2e-factory",
                "base_url": r.base_url,
                "known_public_key": wrong_public_key,
                "priority": 0,
                "enabled": True,
            },
            "servers": [],
        }
    )
    dev.run(f"mkdir -p /etc/nix-upgrade && printf '%s' {shlex.quote(config)} > {SERVERS_JSON}")
    dev.run(
        f"{shlex.quote(_REMOTE_CLI)} register-server --id e2e "
        f"--feed-url {shlex.quote(r.feed_url)} "
        f"--index-public-key {shlex.quote(wrong_public_key)} "
        f"--cache-url {shlex.quote(r.cache_url)} "
        f"--cache-public-key {shlex.quote(wrong_public_key)}"
    )
    if dry_run.get():
        return "tampered registration logged (dry-run)"
    conf = dev.read(f"cat {_NIX_CONF} 2>/dev/null || true")
    require(
        r.cache_public_key not in conf,
        "the good cache key is still trusted — same-name replacement failed",
    )
    return f"wrong key {console.lit(wrong_public_key)} registered, good key gone"


@stage("Plant store witness")
def plant_store_witness(dev: Device) -> str:
    """A file inside the promoted store whose disappearance proves the init
    path wiped rather than reused it (B4/B5)."""
    dev.run(f"touch {_WITNESS}")
    return console.lit(_WITNESS)


@stage("Witness vanished (wipe proof)")
def require_witness_gone(dev: Device) -> str:
    require(
        not dev.read(f"[ -e {_WITNESS} ] && echo yes || true"),
        f"{console.lit(_WITNESS)} survived — the store was reused, not wiped",
    )
    return f"{console.lit(_WITNESS)} gone"


@stage("Plant staging remnants (B3)")
def plant_store_remnants(dev: Device) -> str:
    """Leftovers of an interrupted staged extraction: init must clear both."""
    dev.run(
        f"mkdir -p {_NIX_BACKING}.tmp/junk && touch {_NIX_BACKING}.tmp/junk/f {_NIX_BACKING}.wiped"
    )
    return f"{console.lit(_NIX_BACKING + '.tmp')} + {console.lit(_NIX_BACKING + '.wiped')}"


@stage("Staging remnants gone")
def require_remnants_gone(dev: Device) -> str:
    leftover = dev.read(f"ls -d {_NIX_BACKING}.tmp {_NIX_BACKING}.wiped 2>/dev/null || true")
    require(not leftover, f"staging remnants survived init: {leftover}")
    return "remnants cleared"


@stage("Delete store database (B4)")
def delete_store_db(dev: Device) -> str:
    db = f"{_NIX_BACKING}/var/nix/db/db.sqlite"
    dev.run(f"rm -f {shlex.quote(db)}")
    return f"{console.lit(db)} deleted"


# A /proc/mounts line is "source point fstype opts freq passno"; reading the
# mount point and fstype needs at least this many whitespace-split fields.
_MOUNTS_MIN_FIELDS = 3


def _containing_mount(path: str, mounts: str) -> tuple[str, str] | None:
    """(mount point, fstype) of the mount holding `path`: the longest
    /proc/mounts mount point that is a path-component prefix of `path`
    ("/tmp" contains "/tmp/shm"; a "/tmp/sh" mount does not)."""
    best: tuple[str, str] | None = None
    for line in mounts.splitlines():
        fields = line.split()
        if len(fields) < _MOUNTS_MIN_FIELDS:
            continue
        point, fstype = fields[1], fields[2]
        contains = path == point or path.startswith(point.rstrip("/") + "/")
        if contains and (best is None or len(point) > len(best[0])):
            best = (point, fstype)
    return best


@stage("/dev/shm is tmpfs")
def require_shm_tmpfs(dev: Device) -> str:
    """D1's RAM-backing probe. OpenWRT ships /dev/shm as a SYMLINK to
    /tmp/shm with no dedicated shm mount, so grepping /proc/mounts for a
    ' /dev/shm ' entry can never pass there; resolve the path and judge the
    fstype of the mount containing it instead. The device's busybox has no
    `stat`, so the containment logic runs host-side."""
    resolved = dev.read(f"readlink -f {_SHM_DIR} || echo {_SHM_DIR}") or _SHM_DIR
    entry = _containing_mount(resolved, dev.read("cat /proc/mounts"))
    found = f"containing mount: {entry[0]} ({entry[1]})" if entry else "no containing mount found"
    require(
        entry is not None and entry[1] == "tmpfs",
        f"{console.lit(_SHM_DIR)} (resolves to {console.lit(resolved)}) is not tmpfs-backed"
        f" — {found}",
    )
    return f"{console.lit(resolved)} tmpfs-backed"


@stage("Upload firmware to /dev/shm")
def upload_firmware_shm(dev: Device, image: Image) -> str:
    """D1: stage the image in RAM-backed /dev/shm and flash from there —
    the local-file sysupgrade branch. On OpenWRT both flash-window copies
    (this staging copy and sysupgrade's own /tmp/sysupgrade.img) share
    /tmp's tmpfs size cap (100 MiB on the Deck) — ensure_memory gates free
    RAM, not the cap; with ~23 MiB images the ~46 MiB peak fits."""
    remote = shm_path(image)
    done_if(_remote_sha(dev, remote) == image.sha256)
    # mkdir the RESOLVED dir: on OpenWRT /dev/shm is a symlink (to /tmp/shm,
    # present at boot) — creating the target covers a resolved-target-missing
    # case, where the push's `cat >` would fail with ENOENT.
    dev.run(f'mkdir -p "$(readlink -f {shlex.quote(_SHM_DIR)})"')
    dev.push(image.path, remote)
    if dry_run.get():
        return f"→ {console.lit(remote)}"
    require(
        _remote_sha(dev, remote) == image.sha256,
        f"upload corrupted: {console.lit(image.path.name)} checksum mismatch in {_SHM_DIR}",
    )
    return f"→ {console.lit(remote)} (sha256 verified)"


@stage("Sweep /dev/shm upload")
def sweep_shm_upload(dev: Device, image: Image) -> str:
    dev.run(f"rm -f {shlex.quote(shm_path(image))}")
    return console.lit(shm_path(image))


# Version-agnostic glob (binary-cache-v7.sqlite today) plus the -wal/-shm
# sidecars; deliberately NOT the whole /root/.cache.
_NARINFO_CACHE_GLOB = "/root/.cache/nix/binary-cache-v*.sqlite*"


@stage("Clear nix narinfo cache")
def clear_nix_narinfo_cache(dev: Device) -> str:
    """Remove the device's on-disk narinfo lookup cache: nix records a failed
    substituter query as a NEGATIVE entry (present=0) in
    ~/.cache/nix/binary-cache-v*.sqlite and honors it for
    narinfo-cache-negative-ttl (3600 s by default) — and /root is persistent
    overlay on the Deck, so within the TTL a re-realise never re-queries the
    substituter even after the rig's cache is restored. bmc-nix passes no
    countermeasure to nix-store --realise (product finding, tracked
    separately); the harness deletes the cache files instead."""
    dev.run(f"rm -f {_NARINFO_CACHE_GLOB}")
    return console.lit(_NARINFO_CACHE_GLOB)


@stage("Plant stale next marker (C5)")
def plant_stale_next_marker(dev: Device) -> str:
    """A leftover marker from a hypothetical earlier aborted staging: the
    incoming flash must sweep it when it consumes its own next.*. It MUST be
    a symlink, like a real staged marker (stage_next_boot symlinks then
    renames): the device sweep (sweep_next_markers) mirrors the shell
    activator's `[ -L ]` guard and deliberately skips non-symlinks, so a
    regular file would survive activation and false-fail require_stale_next_gone.
    The target (a generation link like `1-link`) may dangle — the sweep
    removes any non-kept next.* symlink regardless of where it points."""
    dev.run(f"ln -sfn 1-link {_PROFILE_DIR}/{_STALE_NEXT_MARKER}")
    return console.lit(_STALE_NEXT_MARKER)


@stage("Stale next marker swept (C5)")
def require_stale_next_gone(dev: Device) -> str:
    require(
        _STALE_NEXT_MARKER not in _next_markers(dev),
        f"stale {console.lit(_STALE_NEXT_MARKER)} survived activation",
    )
    return f"{console.lit(_STALE_NEXT_MARKER)} gone"


_STORE_BALLAST = f"{_DATA_MOUNT}/.e2e-store-ballast"
_STORE_BALLAST_SPACER = f"{_STORE_BALLAST}.spacer"
# What the fill leaves free. A filesystem at literally zero also fails
# writes the upgrade never makes — logs, the nix db journal — so an abort
# there would not show the realise ran out of space. Must stay under the
# plan's unpacked size, or the flash succeeds and flash_expect_abort says
# the fault did not fire.
_STORE_BALLAST_MARGIN_MIB = 2
# `df -k` columns: Filesystem, 1K-blocks, Used, Available, Use%, Mounted on.
# Counted from the end: df wraps a long device name onto a line of its own,
# and the row that follows then carries five fields, not six.
_DF_AVAILABLE_FIELD = -3
_DF_MIN_FIELDS = 5


def _store_available_kib(dev: Device) -> int:
    out = dev.read(f"df -k {_DATA_MOUNT} | tail -1")
    fields = out.split()
    hint = f"unparseable df output for {console.lit(_DATA_MOUNT)}: {out!r}"
    require(
        len(fields) >= _DF_MIN_FIELDS,
        hint,
    )
    try:
        return int(fields[_DF_AVAILABLE_FIELD])
    except ValueError:
        raise Abort(hint) from None


@stage("Fill the store filesystem (C7)")
def fill_store_filesystem(dev: Device) -> str:
    """Leave the store's filesystem with less room than the incoming
    generation needs, so the realise runs out of space part-way.

    `dd` until it fails is the only way to fill it here.
    The device's busybox ships neither `fallocate` nor `truncate`,
    and a size derived from df's available column would still leave
    ext4's root-reserved blocks for nix, which runs as root, to spend.
    Deleting a pre-sized spacer afterwards is then the only way
    to reopen an exact margin.

    Expect no output for minutes.
    The Deck writes its data partition at roughly 7 MB/s,
    so filling 1.7 GiB took about four minutes."""
    spacer_blocks = _STORE_BALLAST_MARGIN_MIB
    dev.run(f"dd if=/dev/zero of={_STORE_BALLAST_SPACER} bs=1M count={spacer_blocks} 2>/dev/null")
    dev.run(f"dd if=/dev/zero of={_STORE_BALLAST} bs=1M 2>/dev/null || true")
    if not dry_run.get():
        available = _store_available_kib(dev)
        require(
            available == 0,
            f"{console.lit(_DATA_MOUNT)} still has {available} KiB free after ballasting",
        )
    dev.run(f"rm -f {_STORE_BALLAST_SPACER}")
    return f"{console.lit(_DATA_MOUNT)} down to {_STORE_BALLAST_MARGIN_MIB} MiB"


@stage("Sweep the store ballast")
def sweep_store_ballast(dev: Device) -> str:
    """A run killed outright never reaches its teardown, and the ballast it
    leaves fails every later scenario with ENOSPC noise instead of the
    message that scenario expects."""
    dev.run(f"rm -f {_STORE_BALLAST} {_STORE_BALLAST_SPACER}")
    return f"{console.lit(_STORE_BALLAST)} absent"


@stage("Record servers.json (D5)")
def record_servers_json(dev: Device, state: FaultsState) -> str:
    """Snapshot the registry bytes the D5 flash must carry across: the rig
    registration just wrote them, so a post-flash byte match is the
    preservation proof. Existence alone proves nothing — a flash that
    replaced the file with different defaults would still 'exist'."""
    state.servers_json_before = _remote_file_b64(dev, SERVERS_JSON)
    if dry_run.get():
        return "snapshot logged (dry-run: registration was only logged)"
    require(
        state.servers_json_before is not None,
        f"{console.lit(SERVERS_JSON)} missing before the D5 flash — registration did not write it",
    )
    return f"{console.lit(SERVERS_JSON)} snapshot taken"


@stage("Preservation policy (D5)")
def require_preservation_policy(  # noqa: PLR0913  the policy flag, the snapshot, and the injectable clock must meet here
    dev: Device,
    state: FaultsState,
    *,
    servers_json_preserved: bool,
    timeout: int = 30,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> str:
    """First boot into the flashed image: nix.conf is a preserved conffile
    and must survive — checked single-shot, activation rewrites it before
    ssh is even reachable (no race observed). The servers.json half depends
    on the policy:
    - preserved=True (the default): hard assert against the pre-flash
      snapshot — the file must come back byte-identical. Preservation is
      a registered-conffile contract (#BDK-358) proven on hardware
      (BDK-600 dissolved into a harness cleanup bug); it is synchronous
      with the flash, so a present file settles immediately and an absent
      one only gets a short grace poll (filesystem/boot settling) before
      the abort.
    - preserved=False (legacy escape hatch, images predating the conffile
      registration): OBSERVED, never asserted — a single probe reports
      what it saw and passes either way."""
    conf = dev.read(f"cat {_NIX_CONF} 2>/dev/null || true")
    require("experimental-features" in conf, "nix.conf was not preserved across sysupgrade")
    if dry_run.get():
        return "nix.conf preserved; servers.json probe skipped (dry-run)"

    def present() -> bool:
        return bool(dev.read(f"[ -f {SERVERS_JSON} ] && echo yes || true"))

    if not servers_json_preserved:
        observed = (
            "servers.json present (not asserted: --no-servers-json-preserved)"
            if present()
            else "servers.json gone (not asserted: --no-servers-json-preserved)"
        )
        return f"nix.conf preserved; {observed}"
    if state.servers_json_before is None:
        msg = "BUG: servers.json was not recorded before the flash"
        raise RuntimeError(msg)
    start = clock()

    def missing() -> str | None:
        if present():
            return None
        return (
            f"servers.json still missing {clock() - start:.0f}s after the flash "
            f"— contradicts --servers-json-preserved"
        )

    failure = _poll_until(missing, timeout=timeout, sleep=sleep, clock=clock)
    if failure is not None:
        raise Abort(failure)
    require(
        _remote_file_b64(dev, SERVERS_JSON) == state.servers_json_before,
        "servers.json exists but its contents changed across the flash",
    )
    return "nix.conf preserved, servers.json preserved byte-identical"


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
    servers_snapshot: "FileSnapshot | None" = None
    nix_conf_snapshot: "FileSnapshot | None" = None

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
        cycle.server = subprocess.Popen(
            argv,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    _await_upgrade_server(cycle)
    cycle.cache_public_key = (cycle.key_dir / "public").read_text().strip()
    return f"index {console.lit(cycle.index_url)}, cache {console.lit(cycle.cache_url)}"


def _restore_each(*, failed: bool, actions: list[Callable[[], object]]) -> None:
    """Attempt every restore even after one raises, then re-raise the first
    failure: a device left half-restored is the state this exists to prevent.
    Softens to a warning only while a primary failure is unwinding, which
    the restore must not mask.
    """
    if failed:
        for action in actions:
            best_effort(action)
        return
    errors: list[Exception] = []
    for action in actions:
        try:
            action()
        except Exception as e:
            errors.append(e)
    if errors:
        raise errors[0]


@contextmanager
def package_upgrade_session(dev: Device, cycle: UpgradeCycle) -> Iterator[None]:
    """Hand back both files registration writes — the server registry and
    nix.conf, which carries a standing trust grant for the rig's signing key.

    A failed restore fails the run: leaving the rig registered, and every
    production server disabled behind it, must not degrade to a log line.
    """
    failed = False
    try:
        yield
    except BaseException:
        failed = True
        raise
    finally:
        try:
            _restore_each(
                failed=failed,
                actions=[
                    lambda: restore_server_registry(dev, cycle),
                    lambda: restore_nix_conf(dev, cycle),
                ],
            )
        finally:
            stop_upgrade_server(cycle)


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


def _upgrade_server_port_released(host: str, port: int) -> bool:
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            sock.bind((host, port))
    except OSError:
        return False
    return True


def _upgrade_server_ports_released(cycle: UpgradeCycle, *, timeout: float = 10) -> bool:
    deadline = time.monotonic() + timeout
    host = cycle.host or "0.0.0.0"
    while True:
        if _upgrade_server_port_released(host, cycle.port) and _upgrade_server_port_released(
            host, cycle.index_port
        ):
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.1)


def stop_upgrade_server_group(cycle: UpgradeCycle) -> None:
    """Stop the e2e firmware package server and all of its children."""
    server = cycle.server
    if server is None:
        return

    with suppress(ProcessLookupError):
        os.killpg(server.pid, signal.SIGTERM)
    leader_timed_out = False
    if server.poll() is None:
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            leader_timed_out = True
    ports_released = not leader_timed_out and _upgrade_server_ports_released(cycle)
    if not ports_released:
        with suppress(ProcessLookupError):
            os.killpg(server.pid, signal.SIGKILL)
        if server.poll() is None:
            server.wait(timeout=10)

    require(
        _upgrade_server_ports_released(cycle),
        f"upgrade-server ports {cycle.port} and {cycle.index_port} are still occupied",
    )


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
        "--exclusive",
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


@stage("Pre-run package config is the device's own")
def require_unclaimed_package_registry(dev: Device) -> str:
    """Refuse to capture package config an earlier run already claimed.

    A run that dies before its restore leaves the rig registered and every
    production server disabled, or can leave its trusted key in nix.conf
    before the registry write. Capturing either as the pre-run state would
    hand it back at the end as the device's baseline."""

    raw = dev.read(f"if [ -e {SERVERS_JSON} ]; then cat {SERVERS_JSON}; fi")
    servers: list[Any] = []
    if raw.strip():
        try:
            config = json.loads(raw)
        except json.JSONDecodeError as e:
            raise Abort(f"{SERVERS_JSON} is not valid JSON before registration: {e}") from None
        parsed_servers = config.get("servers")
        require(isinstance(parsed_servers, list), f"{SERVERS_JSON} has no servers list")
        servers = parsed_servers

    nix_conf = dev.read(f"if [ -e {_NIX_CONF} ]; then cat {_NIX_CONF}; fi")
    trusted_key_left = False
    for line in nix_conf.splitlines():
        setting, separator, value = line.partition("=")
        if separator and setting.strip() == "extra-trusted-public-keys":
            trusted_key_left = any(
                token.partition(":")[0] == _UPGRADE_SERVER_ID for token in value.split()
            )
            if trusted_key_left:
                break

    registry_left = any(
        isinstance(entry, dict) and entry.get("id") == _UPGRADE_SERVER_ID for entry in servers
    )
    require(
        not registry_left and not trusted_key_left,
        f"an earlier run did not restore the {_UPGRADE_SERVER_ID} package config: capturing "
        f"it would make the rig the device's baseline. Delete its {SERVERS_JSON} entry, "
        "re-enable the servers it disabled, and remove its URL token from extra-substituters "
        "and its dev-upgrade:* token from extra-trusted-public-keys, then re-run.",
    )
    registry_state = "absent" if not raw.strip() else f"no {console.lit(_UPGRADE_SERVER_ID)} entry"
    return f"{console.lit(SERVERS_JSON)} {registry_state}, no stale trust key"


@stage("Only the harness server resolves")
def require_exclusive_package_server(dev: Device) -> str:
    """Prove `register-server --exclusive` took, rather than assume it.

    Any other enabled entry can still decide the upgrade,
    because resolution ranks a candidate's version above its server's priority.
    A `required` public entry the device cannot reach fails the whole probe.
    Registration seeds the registry from the shipped default
    when the runtime file is missing, so such entries appear unprompted."""

    raw = dev.read(f"cat {SERVERS_JSON}")
    try:
        config = json.loads(raw)
    except json.JSONDecodeError as e:
        raise Abort(f"{SERVERS_JSON} is not valid JSON after registration: {e}") from None
    servers = config.get("servers")
    require(isinstance(servers, list), f"{SERVERS_JSON} has no servers list")
    enabled = [
        entry.get("id")
        for entry in servers
        if isinstance(entry, dict) and entry.get("enabled", True)
    ]
    require(
        enabled == [_UPGRADE_SERVER_ID],
        f"{SERVERS_JSON} must leave {_UPGRADE_SERVER_ID} the only enabled server, found {enabled}",
    )
    return f"{console.lit(_UPGRADE_SERVER_ID)} only"


@stage("Authenticate")
def grpc_login(dev: Device, cycle: "GrpcSession") -> str:
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


# ── e2e firmware upgrade ─────────────────────────────────────────────────────────────────


@dataclass
class FirmwareCycle:
    """Mutable carrier threaded through the e2e firmware upgrade stages."""

    password: str
    index_port: int
    stream_deadline: float
    snapshot_dir: Path
    host: str | None = None
    running_version: "BosVersion | None" = None
    image_version: "BosVersion | None" = None
    servers_snapshot: "FileSnapshot | None" = None
    nix_conf_snapshot: "FileSnapshot | None" = None
    opkg_keys_snapshot: "DirSnapshot | None" = None
    bos_version_snapshot: "FileSnapshot | None" = None
    upload_present: bool = False
    mutation_started: bool = False
    bcp_before: frozenset[str] = frozenset()
    init_script_snapshot: "FileSnapshot | None" = None
    bmc_log_offset: int = 0
    pinned_host: str | None = None
    device_identity: str | None = None
    boot_id_before: str | None = None
    cookie: str | None = None
    upgrade_id: str | None = None
    started_upgrade: bool = False


class GrpcSession(Protocol):
    """What grpc_login needs: both UpgradeCycle and FirmwareCycle satisfy it."""

    password: str
    cookie: str | None


class FirmwareIndex(Protocol):
    """Index-server capability needed while checking the firmware offer."""

    def completed(self, path: str) -> bool: ...


@dataclass(frozen=True)
class FileSnapshot:
    remote_path: str
    local: Path | None
    contents: bytes | None = None

    @property
    def present(self) -> bool:
        return self.local is not None or self.contents is not None


@dataclass(frozen=True)
class DirSnapshot:
    remote_path: str
    archive: Path | None


def _remote_size(dev: Device, remote_path: str) -> int:
    return int(dev.read(f"wc -c < {shlex.quote(remote_path)}"))


def _snapshot_bytes(dev: Device, remote_path: str) -> bytes:
    encoded = dev.read(f"base64 < {shlex.quote(remote_path)}")
    return base64.b64decode(encoded)


def _verify_snapshot(dev: Device, remote_path: str, data: bytes) -> None:
    require(
        len(data) == _remote_size(dev, remote_path)
        and hashlib.sha256(data).hexdigest() == _remote_sha(dev, remote_path).lower(),
        f"snapshot of {console.lit(remote_path)} was incomplete or corrupt",
    )


def snapshot_remote_file(dev: Device, remote_path: str, into: Path | None = None) -> FileSnapshot:
    quoted = shlex.quote(remote_path)
    if dev.read(f"test -f {quoted} && echo present || true") != "present":
        return FileSnapshot(remote_path, None)
    data = _snapshot_bytes(dev, remote_path)
    if into is not None:
        into.write_bytes(data)
    _verify_snapshot(dev, remote_path, data)
    return FileSnapshot(remote_path, into, None if into is not None else data)


def restore_remote_file(dev: Device, snap: FileSnapshot) -> None:
    if not snap.present:
        dev.run(f"rm -f {shlex.quote(snap.remote_path)}")
    elif snap.local is not None:
        dev.push(snap.local, RemotePath(snap.remote_path))
    else:
        contents = snap.contents
        if contents is None:
            msg = "BUG: present in-memory snapshot has no contents"
            raise RuntimeError(msg)
        encoded = base64.b64encode(contents).decode()
        temporary = f"{snap.remote_path}.tmp"
        dev.run(
            f"echo {shlex.quote(encoded)} | base64 -d > {shlex.quote(temporary)} "
            f"&& mv {shlex.quote(temporary)} {shlex.quote(snap.remote_path)} && sync"
        )


def _remote_dir_parts(remote_path: str) -> tuple[str, str]:
    parent, _, name = remote_path.rstrip("/").rpartition("/")
    return parent or "/", name


def _remote_keys_archive() -> str:
    return f"/tmp/bmc-e2e-opkg-keys.{os.getpid()}.tar"


def snapshot_remote_dir(dev: Device, remote_path: str, into: Path) -> DirSnapshot:
    quoted = shlex.quote(remote_path)
    if dev.read(f"test -d {quoted} && echo present || true") != "present":
        return DirSnapshot(remote_path, None)
    parent, name = _remote_dir_parts(remote_path)
    remote_archive = _remote_keys_archive()
    try:
        dev.run(
            f"tar -C {shlex.quote(parent)} -cf {shlex.quote(remote_archive)} {shlex.quote(name)}"
        )
        expected_size = _remote_size(dev, remote_archive)
        expected_sha = _remote_sha(dev, remote_archive).lower()
        data = _snapshot_bytes(dev, remote_archive)
        into.write_bytes(data)
        require(
            len(data) == expected_size and hashlib.sha256(data).hexdigest() == expected_sha,
            f"snapshot of {console.lit(remote_path)} was incomplete or corrupt",
        )
        return DirSnapshot(remote_path, into)
    finally:
        dev.run(f"rm -f {shlex.quote(remote_archive)}")


def restore_remote_dir(dev: Device, snap: DirSnapshot) -> None:
    dev.run(f"rm -rf {shlex.quote(snap.remote_path)}")
    if snap.archive is None:
        return
    parent, _ = _remote_dir_parts(snap.remote_path)
    remote_archive = _remote_keys_archive()
    try:
        dev.push(snap.archive, RemotePath(remote_archive))
        dev.run(f"tar -C {shlex.quote(parent)} -xf {shlex.quote(remote_archive)}")
    finally:
        dev.run(f"rm -f {shlex.quote(remote_archive)}")


def _bcp_siblings(dev: Device) -> frozenset[str]:
    pattern = f"{SERVERS_JSON}.bcp*"
    # With no match the glob stays literal and the trailing `[ -e ]` would
    # otherwise make the loop (and ssh) exit 1.
    output = dev.read(
        f'for path in {pattern}; do if [ -e "$path" ]; then basename "$path"; fi; done'
    )
    return frozenset(output.splitlines())


def restore_servers_config(dev: Device, cycle: FirmwareCycle) -> None:
    snapshot = cycle.servers_snapshot
    if snapshot is None:
        msg = "BUG: servers.json was not snapshotted before restoration"
        raise RuntimeError(msg)
    restore_remote_file(dev, snapshot)
    directory = SERVERS_JSON.rpartition("/")[0]
    for name in sorted(_bcp_siblings(dev) - cycle.bcp_before):
        dev.run(f"rm -f {shlex.quote(f'{directory}/{name}')}")


@stage("Preflight versions")
def preflight_versions(dev: Device, image: Image, cycle: FirmwareCycle) -> str:
    running_raw = dev.version
    image_raw = image.version
    try:
        running = parse_bos_version(running_raw)
    except ValueError as e:
        raise Abort(f"malformed device version {running_raw!r}: {e}") from None
    try:
        offered = parse_bos_version(image_raw)
    except ValueError as e:
        raise Abort(f"malformed image version {image_raw!r}: {e}") from None
    cycle.running_version = running
    cycle.image_version = offered
    return f"{console.lit(running.canonical)} → {console.lit(offered.canonical)}"


_BOS_VERSION = "/etc/bos_version"


def anchored_version(running: BosVersion, image: BosVersion) -> BosVersion:
    """Release name strictly below the image's; everything else from `running`."""
    year, month = image.version.year, image.version.month
    if month > 1:
        name = VersionName(year, month - 1, None)
    else:
        if year == 0:
            msg = "cannot decrement release 0.01"
            raise ValueError(msg)
        name = VersionName(year - 1, 12, None)
    return replace(running, version=name)


@stage("Ensure anchor version")
def ensure_anchor_version(dev: Device, cycle: FirmwareCycle) -> str:
    running = cycle.running_version
    offered = cycle.image_version
    if running is None or offered is None:
        msg = "BUG: versions were not resolved before anchoring"
        raise RuntimeError(msg)
    if running.version < offered.version:
        return f"{console.lit(running.canonical)} is already older than the image release"
    anchored = anchored_version(running, offered)
    dev.run(f"printf '%s\\n' {shlex.quote(anchored.canonical)} > {_BOS_VERSION}")
    cycle.running_version = anchored
    return f"{console.lit(running.canonical)} → {console.lit(anchored.canonical)}"


@stage("Preflight device prerequisites")
def preflight_device(dev: Device) -> str:
    require(
        dev.read("command -v base64 >/dev/null && echo ok || true") == "ok",
        "base64 is required on the device",
    )
    require(
        dev.read(f"test -x {shlex.quote(_NIX_CLI)} && echo ok || true") == "ok",
        f"{_NIX_CLI} is missing — prepare the device with deck deploy",
    )
    servers = shlex.quote(SERVERS_JSON)
    if dev.read(f"test -f {servers} && echo present || true") == "present":
        raw = dev.read(f"cat {servers}")
        try:
            json.loads(raw)
        except json.JSONDecodeError as e:
            raise Abort(f"{SERVERS_JSON} is not valid JSON: {e}") from None
    return "base64, nix CLI, and servers config verified"


@stage("Snapshot upgrade config")
def snapshot_upgrade_config(dev: Device, cycle: FirmwareCycle) -> str:
    cycle.bcp_before = _bcp_siblings(dev)
    cycle.servers_snapshot = snapshot_remote_file(
        dev, SERVERS_JSON, cycle.snapshot_dir / "servers.json"
    )
    cycle.nix_conf_snapshot = snapshot_remote_file(dev, _NIX_CONF, cycle.snapshot_dir / "nix.conf")
    return f"snapshots → {console.lit(cycle.snapshot_dir)}"


@stage("Snapshot bos_version")
def snapshot_bos_version(dev: Device, cycle: FirmwareCycle) -> str:
    cycle.bos_version_snapshot = snapshot_remote_file(
        dev, _BOS_VERSION, cycle.snapshot_dir / "bos_version"
    )
    return f"snapshot → {console.lit(cycle.snapshot_dir)}"


@stage("Snapshot opkg keys")
def snapshot_opkg_keys(dev: Device, cycle: FirmwareCycle) -> str:
    cycle.opkg_keys_snapshot = snapshot_remote_dir(
        dev, "/etc/opkg/keys", cycle.snapshot_dir / "opkg-keys.tar"
    )
    return f"snapshot → {console.lit(cycle.snapshot_dir)}"


@stage("Remove uploaded firmware")
def remove_uploaded_image(dev: Device, image: Image, cycle: FirmwareCycle) -> str:
    dev.run(f"rm -f {shlex.quote(image.remote_path)}")
    cycle.upload_present = False
    return console.lit(image.remote_path)


_BMC_KILL_WAIT = 10.0
_BMC_READY_TIMEOUT = 60.0

_BMC_EXE_SUFFIX = "/bin/bmc-openwrt"
_BMC_LOG = "/var/log/bmc/bmc.log"
_INIT_SCRIPT = "/etc/init.d/bmc-compositor"
_ENV_PARAM = "procd_set_param env "
_BMC_RESTART_TIMEOUT = 30.0  # s for procd to replace the service process on restart


def scan_bmc_pids(dev: Device) -> list[int]:
    output = dev.read(
        'for exe in /proc/[0-9]*/exe; do path=$(readlink "$exe") || continue; '
        'case "$path" in */bin/bmc-openwrt) pid=${exe#/proc/}; pid=${pid%/exe}; '
        'printf \'%s\\t%s\\n\' "$pid" "$path";; esac; done'
    )
    pids: list[int] = []
    for line in output.splitlines():
        pid, separator, exe = line.partition("\t")
        if separator and pid.isdigit() and exe.endswith(_BMC_EXE_SUFFIX):
            pids.append(int(pid))
    return pids


def kill_bmc_pids(dev: Device, pids: list[int]) -> None:
    for pid in pids:
        dev.run(f"kill -TERM {pid}")

    deadline = time.monotonic() + _BMC_KILL_WAIT
    remaining = scan_bmc_pids(dev)
    while remaining and time.monotonic() < deadline:
        time.sleep(1)
        remaining = scan_bmc_pids(dev)

    if remaining:
        for pid in remaining:
            dev.run(f"kill -KILL {pid}")
        remaining = scan_bmc_pids(dev)
    require(not remaining, f"bmc-openwrt is still running after KILL: {remaining}")


def quiesce_bmc(dev: Device) -> None:
    dev.run("service bmc-compositor stop")
    pids = scan_bmc_pids(dev)
    if pids:
        kill_bmc_pids(dev, pids)
    require(not scan_bmc_pids(dev), "bmc-openwrt is still running after cleanup")


@stage("Snapshot bmc service script")
def snapshot_service_script(dev: Device, cycle: FirmwareCycle) -> str:
    snap = snapshot_remote_file(dev, _INIT_SCRIPT, cycle.snapshot_dir / "bmc-compositor.init")
    require(snap.local is not None, f"{_INIT_SCRIPT} is missing on the device")
    cycle.init_script_snapshot = snap
    return f"snapshot → {console.lit(str(cycle.snapshot_dir))}"


def _index_env_token(cycle: FirmwareCycle) -> str:
    return f'"BMC_INDEX_URL=http://{cycle.host}:{cycle.index_port}"'


@stage("Restart bmc with index override")
def point_bmc_at_index(dev: Device, cycle: FirmwareCycle) -> str:
    """Inject BMC_INDEX_URL into the procd service env and restart it.

    bmc must stay procd-supervised through the upgrade: an unsupervised bmc
    outlives procd's sysupgrade teardown and observes the sysupgrade child's
    nonzero exit on the success path, turning a completed flash into a
    spurious `Internal: Upgrade failed` on the StartUpgrade stream.
    """
    require(cycle.host is not None, "BUG: index host was not resolved before bmc restart")
    snap = cycle.init_script_snapshot
    if snap is None or snap.local is None:
        msg = "BUG: the service script was not snapshotted before injection"
        raise RuntimeError(msg)
    script = snap.local.read_text()
    require(
        "BMC_INDEX_URL" not in script,
        f"{_INIT_SCRIPT} already carries BMC_INDEX_URL — restore the device first",
    )
    env_lines = [line for line in script.splitlines() if _ENV_PARAM in line]
    require(
        len(env_lines) == 1,
        f"expected exactly one '{_ENV_PARAM.rstrip()}' line in {_INIT_SCRIPT}, "
        f"found {len(env_lines)}",
    )
    edited = script.replace(_ENV_PARAM, f"{_ENV_PARAM}{_index_env_token(cycle)} ", 1)
    cycle.bmc_log_offset = int(dev.read(f"wc -c < {_BMC_LOG} 2>/dev/null || echo 0"))
    dev.run(f"printf '%s' {shlex.quote(edited)} > {_INIT_SCRIPT}")
    dev.run("service bmc-compositor restart")
    expected = _index_env_token(cycle).strip('"')
    deadline = time.monotonic() + _BMC_RESTART_TIMEOUT
    while True:
        pids = scan_bmc_pids(dev)
        if len(pids) == 1:
            environ = dev.read(
                f"tr '\\0' '\\n' < /proc/{pids[0]}/environ 2>/dev/null "
                "| grep '^BMC_INDEX_URL=' || true"
            )
            if environ == expected:
                return f"pid {pids[0]} with {expected}"
        require(
            time.monotonic() < deadline,
            f"restarted bmc did not come up with {expected} within "
            f"{_BMC_RESTART_TIMEOUT:.0f}s; last scan: {pids}",
        )
        time.sleep(0.5)


def strip_index_override(dev: Device, cycle: FirmwareCycle) -> bool:
    """Remove the injected BMC_INDEX_URL token from the on-device service script.

    keep.d preserves /etc/init.d/bmc-compositor across sysupgrade, so the
    injected env var rides the flash onto the new system; the snapshot
    cannot be byte-restored there because it names the old system's store
    path. Returns True when a token was found and removed.
    """
    script = dev.read(f"cat {_INIT_SCRIPT}")
    injected = f"{_ENV_PARAM}{_index_env_token(cycle)} "
    if injected not in script:
        require(
            "BMC_INDEX_URL" not in script,
            f"{_INIT_SCRIPT} carries an unexpected BMC_INDEX_URL entry",
        )
        return False
    dev.run(f"printf '%s' {shlex.quote(script.replace(injected, _ENV_PARAM, 1))} > {_INIT_SCRIPT}")
    return True


def bmc_log_tail(dev: Device, cycle: FirmwareCycle) -> str:
    first_byte = cycle.bmc_log_offset + 1
    return dev.read(f"tail -c +{first_byte} {_BMC_LOG} 2>/dev/null || true")


@stage("Wait for bmc gRPC")
def await_bmc_ready(dev: Device, cycle: FirmwareCycle) -> str:
    deadline = time.monotonic() + _BMC_READY_TIMEOUT
    while True:
        try:
            remaining = deadline - time.monotonic()
            require(
                remaining > 0,
                f"bmc gRPC did not answer within {_BMC_READY_TIMEOUT:.0f}s; "
                f"new bmc log:\n{bmc_log_tail(dev, cycle) or '(no new log output)'}",
            )
            subprocess.run(
                _grpcurl_argv(
                    dev,
                    "AuthenticationService/Login",
                    data={"password": cycle.password},
                    cookie=None,
                    max_time=max(1, int(remaining)),
                ),
                capture_output=True,
                text=True,
                check=True,
                timeout=remaining,
            )
            return "gRPC ready"
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            require(
                time.monotonic() < deadline,
                f"bmc gRPC did not answer within {_BMC_READY_TIMEOUT:.0f}s; "
                f"new bmc log:\n{bmc_log_tail(dev, cycle) or '(no new log output)'}",
            )
            time.sleep(min(1, max(0, deadline - time.monotonic())))


_FIRMWARE_PHASE_PREFIX = "FIRMWARE_UPGRADE_PHASE_"


@stage("Require auto-upgrade disabled")
def require_auto_upgrade_disabled(dev: Device, session: GrpcSession) -> str:
    response = _grpcurl(dev, "UpgradeService/GetAutoUpgrade", cookie=session.cookie)
    # grpcurl omits proto3 default-valued fields, so a disabled schedule
    # arrives without any "enabled" key at all.
    require(
        not response.get("enabled"),
        "auto-upgrade is enabled — disable it via SetAutoUpgrade or the web UI first",
    )
    return "auto-upgrade disabled"


@stage("Check for firmware upgrade")
def check_for_firmware_upgrade(
    dev: Device,
    image: Image,
    cycle: FirmwareCycle,
    index: FirmwareIndex,
) -> str:
    running_version = cycle.running_version
    image_version = cycle.image_version
    if running_version is None or image_version is None:
        msg = "BUG: firmware versions were not resolved before CheckForUpgrade"
        raise RuntimeError(msg)

    response = _grpcurl(dev, "UpgradeService/CheckForUpgrade", cookie=cycle.cookie)
    firmware = response.get("firmware")
    if not isinstance(firmware, dict):
        raise Abort(
            f"no firmware upgrade offered for running release {running_version.version} "
            f"and image release {image_version.version}; response: {json.dumps(response)}"
        )

    require(
        firmware.get("version") == image_version.canonical,
        f"firmware version does not match image: {firmware.get('version')}",
    )
    offered_hash = firmware.get("hash")
    require(
        isinstance(offered_hash, str) and offered_hash.lower() == image.sha256.lower(),
        f"firmware hash does not match image: {offered_hash}",
    )
    try:
        offered_size = int(firmware.get("fileSizeBytes"))
    except (TypeError, ValueError):
        raise Abort(f"firmware size is invalid: {firmware.get('fileSizeBytes')}") from None
    require(offered_size == image.size, f"firmware size does not match image: {offered_size}")
    require(
        response.get("disruption") == "UPGRADE_DISRUPTION_REBOOT",
        f"expected a REBOOT disruption, got {response.get('disruption')}",
    )
    upgrade_id = response.get("upgradeId")
    require(
        isinstance(upgrade_id, str) and bool(upgrade_id),
        "firmware offer returned no upgrade id",
    )
    require(
        index.completed("/index.v1.json"),
        "firmware offer was returned without a completed /index.v1.json fetch",
    )
    cycle.upgrade_id = upgrade_id
    return f"{console.lit(image_version.canonical)} ({console.human_size(image.size)})"


class StreamOutcome(enum.Enum):
    PROVISIONAL_SUCCESS = enum.auto()
    REJECTED = enum.auto()
    TERMINAL_FAILURE = enum.auto()
    POSSIBLY_ACCEPTED = enum.auto()


@dataclass(frozen=True)
class StreamResult:
    events: list[dict[str, Any]]
    exit_code: int
    status_code: str | None
    status_message: str | None
    stderr: str


_GRPC_STATUS_NAMES = {
    0: "Ok",
    1: "Cancelled",
    2: "Unknown",
    3: "InvalidArgument",
    4: "DeadlineExceeded",
    5: "NotFound",
    6: "AlreadyExists",
    7: "PermissionDenied",
    8: "ResourceExhausted",
    9: "FailedPrecondition",
    10: "Aborted",
    11: "OutOfRange",
    12: "Unimplemented",
    13: "Internal",
    14: "Unavailable",
    15: "DataLoss",
    16: "Unauthenticated",
}


def run_firmware_stream(dev: Device, cycle: FirmwareCycle) -> StreamResult:
    upgrade_id = cycle.upgrade_id
    if upgrade_id is None:
        msg = "BUG: CheckForUpgrade did not run before StartUpgrade"
        raise RuntimeError(msg)
    argv = _grpcurl_argv(
        dev,
        "UpgradeService/StartUpgrade",
        data={"upgradeId": upgrade_id},
        cookie=cycle.cookie,
        max_time=None,
    )
    argv[2:2] = ["-format-error", "-max-time", str(cycle.stream_deadline)]

    cycle.started_upgrade = True
    try:
        proc = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    except OSError:
        cycle.started_upgrade = False
        raise

    stdout = proc.stdout
    if stdout is None:
        msg = "BUG: Popen(stdout=PIPE) produced no stdout"
        raise RuntimeError(msg)
    stderr_pipe = proc.stderr
    stderr_chunks: list[str] = []
    stderr_thread: threading.Thread | None = None
    if stderr_pipe is not None:
        stderr_thread = threading.Thread(
            target=lambda: stderr_chunks.append(stderr_pipe.read()),
            daemon=True,
        )
        stderr_thread.start()

    decoder = json.JSONDecoder()
    events: list[dict[str, Any]] = []
    remainder = ""
    for line in stdout:
        remainder = _drain_events(decoder, remainder + line, events)
    exit_code = proc.wait()
    if stderr_thread is not None:
        stderr_thread.join()
    stderr = "".join(stderr_chunks)

    status_code = None
    status_message = None
    stream_events: list[dict[str, Any]] = []
    for event in events:
        status = _grpc_status(event)
        if status is None:
            stream_events.append(event)
        else:
            status_code, status_message = status
    if exit_code != 0 and status_code is None:
        status_code, status_message = _grpc_status_from_text(stderr)
    if remainder:
        stderr = f"{stderr.rstrip()}\nincomplete stdout JSON: {remainder}".lstrip()

    return StreamResult(stream_events, exit_code, status_code, status_message, stderr)


def _grpc_status_from_text(text: str) -> tuple[str | None, str | None]:
    decoder = json.JSONDecoder()
    for offset, char in enumerate(text):
        if char != "{":
            continue
        try:
            value, _ = decoder.raw_decode(text[offset:])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and (status := _grpc_status(value)) is not None:
            return status
    return None, None


def _grpc_status(value: dict[str, Any]) -> tuple[str, str | None] | None:
    candidate = value.get("error", value)
    if not isinstance(candidate, dict) or "code" not in candidate:
        return None
    raw_code = candidate["code"]
    if isinstance(raw_code, int):
        code = _GRPC_STATUS_NAMES.get(raw_code)
    elif isinstance(raw_code, str):
        code = _normalize_grpc_status(raw_code)
    else:
        code = None
    if code is None:
        return None
    message = candidate.get("message")
    return code, message if isinstance(message, str) else None


def _normalize_grpc_status(code: str) -> str:
    words = code.removeprefix("CODE_").lower().split("_")
    return "".join(word.capitalize() for word in words)


def classify_stream(result: StreamResult) -> StreamOutcome:
    firmware_events = [
        event
        for event in result.events
        if isinstance(event.get("firmwarePhase"), str)
        and event["firmwarePhase"].startswith(_FIRMWARE_PHASE_PREFIX)
    ]
    if any("finished" in event for event in result.events) or result.status_code in {
        "Internal",
        "FailedPrecondition",
    }:
        outcome = StreamOutcome.TERMINAL_FAILURE
    elif result.status_code == "Unavailable" and not firmware_events:
        outcome = StreamOutcome.REJECTED
    elif result.status_code in {"DeadlineExceeded", "Cancelled"}:
        outcome = StreamOutcome.POSSIBLY_ACCEPTED
    else:
        download_index = next(
            (
                index
                for index, event in enumerate(result.events)
                if event.get("firmwarePhase") == f"{_FIRMWARE_PHASE_PREFIX}DOWNLOADING"
            ),
            -1,
        )
        progress_index = (
            next(
                (
                    index
                    for index, event in enumerate(result.events)
                    if index > download_index and isinstance(event.get("download"), dict)
                ),
                -1,
            )
            if download_index >= 0
            else -1
        )
        verifying_index = (
            next(
                (
                    index
                    for index, event in enumerate(result.events)
                    if index > progress_index
                    and event.get("firmwarePhase") == f"{_FIRMWARE_PHASE_PREFIX}VERIFYING"
                ),
                -1,
            )
            if progress_index >= 0
            else -1
        )
        download_evidence = verifying_index >= 0
        applying_after_verify = any(
            index > verifying_index
            and event.get("firmwarePhase") == f"{_FIRMWARE_PHASE_PREFIX}APPLYING"
            for index, event in enumerate(result.events)
        )
        clean_success = result.exit_code == 0 and download_evidence and applying_after_verify
        severed_success = (
            result.exit_code != 0
            and download_evidence
            and result.status_code in {None, "Unavailable"}
        )
        outcome = (
            StreamOutcome.PROVISIONAL_SUCCESS
            if clean_success or severed_success
            else StreamOutcome.POSSIBLY_ACCEPTED
        )
    return outcome


BOOT_POLL_TIMEOUT = 180.0

_BOOT_ID_PATH = "/proc/sys/kernel/random/boot_id"
# Populated at boot from OTP/NVMEM (bos-defaults.sh WIFI_MAC_PATH). Present on
# production boards, where the device-tree `serial-number` node is not, and
# stable across a reflash since it derives from fused silicon.
_DEVICE_ID_PATH = "/tmp/wifi_mac"


class DeviceIdentityError(Abort):
    """The reconnect reached a different physical Deck."""


def _read_device_identity(dev: Device) -> str:
    return dev.read(f"cat {_DEVICE_ID_PATH}").strip("\0 \t\n\r\v\f").casefold()


@stage("Snapshot device identity")
def snapshot_device_identity(dev: Device, cycle: FirmwareCycle) -> str:
    identity = _read_device_identity(dev)
    require(bool(identity), "device identity is empty")
    cycle.device_identity = identity
    return console.lit(identity)


def verify_device_identity(dev: Device, cycle: FirmwareCycle) -> str:
    expected = cycle.device_identity
    if expected is None:
        msg = "BUG: device identity was not snapshotted before reconnect"
        raise RuntimeError(msg)
    identity = _read_device_identity(dev)
    if identity != expected:
        raise DeviceIdentityError(
            f"device identity changed: expected {console.lit(expected)}, "
            f"got {console.lit(identity)}"
        )
    return identity


@stage("Snapshot boot id")
def snapshot_boot_id(dev: Device, cycle: FirmwareCycle) -> str:
    boot_id = dev.read(f"cat {_BOOT_ID_PATH}")
    cycle.boot_id_before = boot_id
    return console.lit(boot_id)


def poll_boot_id_change(
    dev: Device,
    cycle: FirmwareCycle,
    *,
    timeout: float,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> bool:
    boot_id_before = cycle.boot_id_before
    if boot_id_before is None:
        msg = "BUG: boot id was not snapshotted before the reboot poll"
        raise RuntimeError(msg)

    deadline = clock() + timeout
    while clock() < deadline:
        try:
            verify_device_identity(dev, cycle)
            boot_id = dev.read(f"cat {_BOOT_ID_PATH}")
        except (subprocess.CalledProcessError, OSError):
            pass
        else:
            if boot_id != boot_id_before:
                return True

        remaining = deadline - clock()
        if remaining <= 0:
            break
        sleep(min(1.0, remaining))
    return False


def read_flashed_version(dev: Device) -> BosVersion:
    return parse_bos_version(dev.read("cat /etc/bos_version"))


@stage("Verify stock service")
def verify_stock_service(dev: Device) -> str:
    running = dev.read("service bmc-compositor status >/dev/null 2>&1 && echo running || true")
    require(
        running == "running",
        "bmc-compositor is not running as the stock procd service",
    )
    return "bmc-compositor running"


@stage("Restore config on fresh boot")
def restore_after_success(dev: Device, cycle: FirmwareCycle) -> str:
    nix_conf_snapshot = cycle.nix_conf_snapshot
    if nix_conf_snapshot is None:
        msg = "BUG: nix.conf was not snapshotted before restoration"
        raise RuntimeError(msg)

    quiesce_bmc(dev)
    restore_servers_config(dev, cycle)
    restore_remote_file(dev, nix_conf_snapshot)
    cycle.opkg_keys_snapshot = None
    stripped = strip_index_override(dev, cycle)
    dev.run("service bmc-compositor start")
    verify_stock_service(dev)
    suffix = ", index override stripped" if stripped else ""
    return f"servers.json and nix.conf restored{suffix}"

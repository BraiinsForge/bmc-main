"""Reusable deploy stages, composed by the procedure scripts.

Each stage is a guarded function (see bmc_tui.stage). Device access goes through
the read/run seam, so `--dry-run` skips mutations while read-only checks still
run. The authoritative firmware compatibility check runs on the device during
sysupgrade; this catalog only fails fast on the obvious local problems.
"""

import difflib
import json
import shlex
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from bmc_tui import console
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.nix import Built, Nix, Pkg
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
_NIX_BACKING = "/mnt/data/nix"
_INIT_TARBALL = ".#init-tarball-armv7"


@stage("Device reachable")
def ensure_device_reachable(dev: Device) -> None:
    require(
        dev.reachable,
        f"{dev.host} is unreachable — power-cycle the Deck and check the network",
    )


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
    require(
        available >= need,
        f"need {console.human_size(need)} free RAM, only {console.human_size(available)} available",
    )
    return f"{console.lit(console.human_size(available))} RAM available"


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
def sysupgrade(dev: Device, image: Image, *, force: bool = False, assume_yes: bool = False) -> str:
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
    dev.run(f"sysupgrade {flag}{image.remote_path}", expect_disconnect=True)
    return f"{console.lit(image.version)} → reboot"


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

    attrs: list[str]  # flake attrs to deploy; empty → discover core, bmc-nix-cli + all widgets
    prefix: str = _DECK_PACKAGES  # attr root for the build profile (see package_prefix)
    resolved: list[Pkg] = field(default_factory=list)
    built: list[Built] = field(default_factory=list)


@stage("bmc-nix-cli present")
def ensure_nix_cli(nix: Nix, dev: Device) -> None:
    def present() -> bool:
        return "ok" in dev.read(f"test -x {_NIX_CLI} && echo ok || true")

    def bootstrap() -> None:
        [built] = nix.build([nix.resolve(".#deck-packages.bmc-nix-cli")])
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


def _qualify(attr: str, prefix: str) -> str:
    """Expand a bare package name to its `prefix.` attr (profile-aware)."""
    return attr if "#" in attr else f"{prefix}.{attr}"


@stage("Resolve packages")
def resolve_packages(nix: Nix, plan: Deployment) -> str:
    if not plan.attrs:
        names = ["core", "bmc-nix-cli", *nix.discover_widgets()]
        plan.attrs = [f"{plan.prefix}.{name}" for name in names]
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


@stage("Register in bmc profile")
def register_packages(dev: Device, plan: Deployment) -> str:
    out = dev.run(_register_cmd(plan.built))
    names = ", ".join(f"{console.lit(b.name)} {b.version}" for b in plan.built)
    generation = _generation_number(out)
    return f"{names} → generation {console.lit(generation)}" if generation else names


@stage("Restart compositor")
def restart_compositor(dev: Device) -> str:
    """Offer to restart the compositor so it reloads the widget set."""

    if dry_run.get():
        return "skipped (dry-run)"
    if not console.confirm("Restart the compositor now to load new or changed widgets?"):
        return "skipped"
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


@stage("Nix store absent")
def ensure_store_absent(dev: Device) -> str:
    """Refuse to reinitialise over a populated store; hand back the clear-down."""

    remedy = f"ssh {dev.login} 'umount /nix 2>/dev/null; rm -rf /mnt/data/nix /nix'"
    require(
        not _store_populated(dev),
        f"{console.lit(_NIX_STORE)} or {console.lit(_NIX_BACKING)} already populated — "
        f"to reinitialise, first clear them: {console.lit(remedy)}",
    )
    return "store is clean"


@stage("Bind-mount /nix")
def mount_nix_store(dev: Device) -> str:
    done_if(_nix_mounted(dev))
    dev.run(f"mkdir -p {_NIX_BACKING} {_NIX_STORE} && mount --bind {_NIX_BACKING} {_NIX_STORE}")
    return f"{console.lit(_NIX_BACKING)} → {console.lit(_NIX_STORE)}"


@stage("Build init tarball")
def build_init_tarball(nix: Nix, plan: Provisioning) -> str:
    """Build the init tarball and locate its single `.tar.gz`."""

    out = nix.build_out(_INIT_TARBALL)
    tarballs = sorted(Path(out).glob("*.tar.gz"))
    require(
        len(tarballs) == 1,
        f"expected one .tar.gz in {console.lit(out)}, found {len(tarballs)}",
    )
    plan.tarball = tarballs[0]
    size = console.human_size(plan.tarball.stat().st_size)
    return f"{console.lit(plan.tarball.name)} ({console.lit(size)})"


@stage("Stream init tarball")
def stream_init_tarball(dev: Device, plan: Provisioning) -> str:
    tarball = plan.tarball
    if tarball is None:
        msg = "BUG: init tarball was not built before the stream stage"
        raise RuntimeError(msg)
    dev.extract_tar(tarball)
    return f"extracted {console.lit(tarball.name)} → {console.lit('/')}"


@stage("Activate profile")
def activate_profile(dev: Device) -> str:
    """Activate generation 1 via its entrypoint, wiring up the device store."""

    dev.run(f"{_PROFILE_DIR}/1-link/core/activation/entrypoint")
    return f"activated {console.lit('generation 1')}"


def _store_populated(dev: Device) -> bool:
    """True if either store dir has contents; absent or empty both pass."""

    listing = dev.read(
        f'for d in {_NIX_STORE} {_NIX_BACKING}; do [ -d "$d" ] && ls -A "$d" 2>/dev/null; done'
    )
    return bool(listing)


def _nix_mounted(dev: Device) -> bool:
    """True if /nix is mounted, per /proc/mounts."""

    return bool(dev.read("grep ' /nix ' /proc/mounts || true"))


def _mem_available(dev: Device) -> int:
    """Free RAM in bytes; /tmp is swapless tmpfs, so RAM bounds upload+flash."""

    kb = dev.read("awk '/^MemAvailable:/ {print $2}' /proc/meminfo")
    return int(kb) * 1024


def _remote_sha(dev: Device, remote_path: str) -> str:
    """Hex sha256 of the on-device file; empty when absent, so never a false match."""

    return dev.read(f"sha256sum {remote_path} 2>/dev/null | cut -d' ' -f1")


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
        "--base-url",
        cycle.index_url,
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
    """Package names from a ListInstallableWidgets response."""
    return [w["packageName"] for w in response.get("widgets", [])]


def _installed_package_names(list_packages_json: dict[str, Any]) -> list[str]:
    """Package names from `bmc-nix-cli list-packages --json`."""
    return [p["name"] for p in list_packages_json.get("packages", [])]


@stage("Remove widget for reinstall")
def remove_package(dev: Device, cycle: UpgradeCycle, widget: str) -> str:
    dev.run(
        f"PATH=/run/current-profile/bin:$PATH {shlex.quote(_NIX_CLI)} "
        f"remove-packages --name {shlex.quote(widget)}"
    )
    return f"removed {widget}"


@stage("List installable widgets")
def list_installable_widgets(dev: Device, cycle: UpgradeCycle, widget: str) -> str:
    response = _grpcurl(dev, "UpgradeService/ListInstallableWidgets", cookie=cycle.cookie)
    names = _installable_widget_names(response)
    if widget not in names:
        raise Abort(f"{widget} not offered as installable; got {names}")
    entry = next(w for w in response["widgets"] if w["packageName"] == widget)
    for key in ("uid", "displayName", "category", "icon"):
        if not entry.get(key):
            raise Abort(f"{widget} missing {key} in discovery: {entry}")
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
def verify_widget_installed(dev: Device, widget: str) -> str:
    raw = dev.read(
        f"PATH=/run/current-profile/bin:$PATH {shlex.quote(_NIX_CLI)} list-packages --json"
    )
    installed = _installed_package_names(json.loads(raw))
    if widget not in installed:
        raise Abort(f"{widget} not in list-packages after install: {installed}")
    return f"{widget} present in profile"


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

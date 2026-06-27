"""Reusable deploy stages, composed by the procedure scripts.

Each stage is a guarded function (see bmc_tui.stage). Device access goes through
the read/run seam, so `--dry-run` skips mutations while read-only checks still
run. The authoritative firmware compatibility check runs on the device during
sysupgrade; this catalog only fails fast on the obvious local problems.
"""

import difflib
import shlex
import subprocess
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

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

    attrs: list[str]  # flake attrs to deploy; empty → discover core + all widgets
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
        plan.attrs = [f"{plan.prefix}.{name}" for name in ["core", *nix.discover_widgets()]]
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

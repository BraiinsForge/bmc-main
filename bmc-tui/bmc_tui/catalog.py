"""Reusable deploy stages, composed by the procedure scripts.

Each stage is a guarded function (see bmc_tui.stage). Device access goes through
the read/run seam, so `--dry-run` skips mutations while read-only checks still
run. The authoritative firmware compatibility check runs on the device during
sysupgrade; this catalog only fails fast on the obvious local problems.
"""

import time
from collections.abc import Callable

from bmc_tui import console
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.stage import done_if, ensure, require, stage

_NIX_CONF = "/etc/nix/nix.conf"
_EXPERIMENTAL = "experimental-features = nix-command flakes"


@stage("Device reachable")
def ensure_device_reachable(dev: Device) -> None:
    require(
        dev.reachable,
        f"{dev.host} is unreachable — power-cycle the Deck and check the network",
    )


@stage("nix.conf experimental-features")
def ensure_nix_conf(dev: Device) -> None:
    ensure(
        lambda: "experimental-features" in dev.read(f"cat {_NIX_CONF} 2>/dev/null"),
        lambda: dev.run(f"mkdir -p /etc/nix && printf '{_EXPERIMENTAL}\\n' > {_NIX_CONF}"),
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


@stage("Free space on /mnt/data")
def ensure_free_space(dev: Device, remote_dir: str, need: int) -> str:
    free = _free_bytes(dev, remote_dir)
    require(
        free >= need,
        f"need {console.human_size(need)} on {remote_dir}, only {console.human_size(free)} free",
    )
    return f"{console.lit(console.human_size(free))} free"


@stage("Upload firmware")
def upload_firmware(dev: Device, image: Image) -> str:
    done_if(_remote_size(dev, image.remote_path) == image.size)
    dev.push(image.path, image.remote_path)
    return f"uploaded to {console.lit(image.remote_path)}"


@stage("Sysupgrade")
def sysupgrade(dev: Device, image: Image, *, force: bool = False) -> str:
    done_if(dev.version == image.version)
    flag = "-F " if force else ""
    dev.run(f"sysupgrade {flag}{image.remote_path}", expect_disconnect=True)
    return "flashing — the device will reboot"


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


def _free_bytes(dev: Device, remote_dir: str) -> int:
    # `df -k` → last line, column 4 (Available, in 1K blocks).
    available_kb = dev.read(f"df -k {remote_dir}").splitlines()[-1].split()[3]
    return int(available_kb) * 1024


def _remote_size(dev: Device, remote_path: str) -> int:
    # `du -b` matches the firmware's own COMMAND; -1 when the file is absent.
    raw = dev.read(f"du -b {remote_path} 2>/dev/null | cut -f1")
    return int(raw or "-1")


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

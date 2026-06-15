"""Unit tests for the deploy stage catalog."""

import io
import subprocess
import tarfile
from pathlib import Path

import pytest

from bmc_tui import catalog
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.stage import Abort

_TARGET = "stm32mp15/ii3"
_TOP = "sysupgrade-stm32mp15_ii3-emmc"


def _cp(argv: list[str], stdout: str = "") -> "subprocess.CompletedProcess[str]":
    return subprocess.CompletedProcess(argv, 0, stdout=stdout, stderr="")


class _Router:
    """Fake Runner mapping an ssh command substring to canned stdout."""

    def __init__(self, routes: dict[str, str]) -> None:
        self.routes = routes
        self.calls: list[list[str]] = []

    def __call__(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        self.calls.append(argv)
        cmd = argv[-1] if argv and argv[0] == "ssh" else " ".join(argv)
        for key, value in self.routes.items():
            if key in cmd:
                return _cp(argv, value)
        return _cp(argv)


def _unreachable(argv: list[str]) -> "subprocess.CompletedProcess[str]":
    raise subprocess.CalledProcessError(255, argv)


def _image(tmp_path: Path, *, top: str = _TOP, extra: tuple[str, ...] = ("rootfs.img",)) -> Image:
    fw = tmp_path / "fw.tar"
    with tarfile.open(fw, "w") as tar:
        files = {"COMMAND": b'UPGRADE_FW_VERSION="2026-06-14-x"\n', **{n: b"x" for n in extra}}
        for name, data in files.items():
            info = tarfile.TarInfo(f"{top}/{name}")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    return Image(fw)


# ── ensure_device_reachable ───────────────────────────────────────────────────


def test_reachable_ok() -> None:
    catalog.ensure_device_reachable(Device("h", runner=_Router({})))


def test_reachable_aborts_when_unreachable() -> None:
    with pytest.raises(Abort, match="unreachable"):
        catalog.ensure_device_reachable(Device("h", runner=_unreachable))


# ── ensure_nix_conf ───────────────────────────────────────────────────────────


def test_nix_conf_noop_when_present() -> None:
    def runner(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        if "cat /etc/nix/nix.conf" in argv[-1]:
            return _cp(argv, "experimental-features = nix-command flakes\n")
        raise AssertionError("remedy must not run when nix.conf is already set")

    catalog.ensure_nix_conf(Device("h", runner=runner))


def test_nix_conf_writes_when_absent() -> None:
    state = {"conf": ""}

    def runner(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "cat /etc/nix/nix.conf" in cmd:
            return _cp(argv, state["conf"])
        if "printf" in cmd:
            state["conf"] = "experimental-features = nix-command flakes\n"
        return _cp(argv)

    catalog.ensure_nix_conf(Device("h", runner=runner))
    assert "experimental-features" in state["conf"]


# ── validate_firmware_image ───────────────────────────────────────────────────


def test_validate_accepts_matching_image(tmp_path: Path) -> None:
    catalog.validate_firmware_image(_image(tmp_path), device_target=_TARGET)


def test_validate_rejects_non_sysupgrade(tmp_path: Path) -> None:
    with pytest.raises(Abort, match="not a Deck sysupgrade image"):
        catalog.validate_firmware_image(_image(tmp_path, extra=()), device_target=_TARGET)


def test_validate_rejects_wrong_board_family(tmp_path: Path) -> None:
    with pytest.raises(Abort, match="wrong board family"):
        catalog.validate_firmware_image(_image(tmp_path), device_target="am335x/evm")


def test_validate_aborts_on_missing_image(tmp_path: Path) -> None:
    missing = Image(tmp_path / "nope.tar")
    with pytest.raises(Abort, match="not found"):
        catalog.validate_firmware_image(missing, device_target=_TARGET)


# ── ensure_free_space ─────────────────────────────────────────────────────────

_DF = (
    "Filesystem 1K-blocks Used Available Use% Mounted\n"
    "/dev/mmcblk0p4 2902528 1145000 1611000 42% /mnt/data"
)


def test_free_space_ok() -> None:
    dev = Device("h", runner=_Router({"df -k": _DF}))
    catalog.ensure_free_space(dev, "/mnt/data", 1_000_000)  # 1.6 GB free


def test_free_space_aborts_when_insufficient() -> None:
    dev = Device("h", runner=_Router({"df -k": _DF}))
    with pytest.raises(Abort, match="only"):
        catalog.ensure_free_space(dev, "/mnt/data", 5_000_000_000)


# ── upload_firmware ───────────────────────────────────────────────────────────


def test_upload_pushes_when_absent(tmp_path: Path) -> None:
    image = _image(tmp_path)
    router = _Router({"du -b": ""})  # absent → -1 != size
    catalog.upload_firmware(Device("h", runner=router), image)
    assert any(call[0] == "scp" for call in router.calls)


def test_upload_skips_when_already_uploaded(tmp_path: Path) -> None:
    image = _image(tmp_path)
    router = _Router({"du -b": str(image.size)})
    dev = Device("h", runner=router)
    catalog.upload_firmware(dev, image)
    assert not any(call[0] == "scp" for call in router.calls)  # skipped


# ── sysupgrade ────────────────────────────────────────────────────────────────


def test_sysupgrade_skips_when_already_on_target(tmp_path: Path) -> None:
    image = _image(tmp_path)
    router = _Router({"cat /etc/bos_version": image.version})
    dev = Device("h", runner=router)
    catalog.sysupgrade(dev, image)
    assert not any("sysupgrade" in c[-1] for c in router.calls if c[0] == "ssh")


def test_sysupgrade_runs_with_force(tmp_path: Path) -> None:
    image = _image(tmp_path)
    router = _Router({"cat /etc/bos_version": "older-version"})
    dev = Device("h", runner=router)
    catalog.sysupgrade(dev, image, force=True)
    cmds = [c[-1] for c in router.calls if c[0] == "ssh"]
    assert any("sysupgrade -F " in c for c in cmds)


# ── wait_for_device ───────────────────────────────────────────────────────────


def test_wait_for_device_ok() -> None:
    catalog.wait_for_device(Device("h", runner=_Router({})), timeout=0)


def test_wait_for_device_times_out() -> None:
    with pytest.raises(Abort, match="did not return"):
        catalog.wait_for_device(Device("h", runner=_unreachable), timeout=0)


# ── verify_post_upgrade ───────────────────────────────────────────────────────

_GOOD = {
    "cat /etc/bos_version": "2026-06-14-x",
    "mount": "/dev/mmcblk0p4 on /nix type ext4 (rw)",
    "cat /etc/nix/nix.conf": "experimental-features = nix-command flakes",
}


def test_verify_ok() -> None:
    catalog.verify_post_upgrade(Device("h", runner=_Router(_GOOD)), expect="2026-06-14-x")


def test_verify_aborts_on_version_mismatch() -> None:
    dev = Device("h", runner=_Router(_GOOD))
    with pytest.raises(Abort, match="expected"):
        catalog.verify_post_upgrade(dev, expect="some-other-version")


def test_verify_aborts_when_nix_unmounted() -> None:
    routes = {**_GOOD, "mount": "/dev/mmcblk0p4 on / type ext4 (rw)"}
    with pytest.raises(Abort, match="/nix is not mounted"):
        catalog.verify_post_upgrade(Device("h", runner=_Router(routes)), expect="2026-06-14-x")

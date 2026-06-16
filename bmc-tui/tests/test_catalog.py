"""Unit tests for the deploy stage catalog."""

import io
import subprocess
import tarfile
from collections.abc import Callable, Iterable
from pathlib import Path

import pytest

from bmc_tui import catalog
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.stage import Abort, dry_run

_TARGET = "stm32mp15/ii3"
_TOP = "sysupgrade-stm32mp15_ii3-emmc"

_Respond = Callable[[list[str]], "subprocess.CompletedProcess[str]"]


def _cp(argv: list[str], stdout: str = "") -> "subprocess.CompletedProcess[str]":
    return subprocess.CompletedProcess(argv, 0, stdout=stdout, stderr="")


class _Exec:
    """Fake Exec: run() delegates to `respond(argv)`; stream() records bytes."""

    def __init__(self, respond: _Respond) -> None:
        self._respond = respond
        self.runs: list[list[str]] = []
        self.streams: list[tuple[list[str], bytes]] = []

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        self.runs.append(argv)
        return self._respond(argv)

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        self.streams.append((argv, b"".join(chunks)))


def _routes(routes: dict[str, str]) -> _Respond:
    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1] if argv and argv[0] == "ssh" else " ".join(argv)
        for key, value in routes.items():
            if key in cmd:
                return _cp(argv, value)
        return _cp(argv)

    return respond


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
    catalog.ensure_device_reachable(Device("h", backend=_Exec(_routes({}))))


def test_reachable_aborts_when_unreachable() -> None:
    with pytest.raises(Abort, match="unreachable"):
        catalog.ensure_device_reachable(Device("h", backend=_Exec(_unreachable)))


# ── ensure_nix_conf ───────────────────────────────────────────────────────────


def test_nix_conf_noop_when_present() -> None:
    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        if "cat /etc/nix/nix.conf" in argv[-1]:
            return _cp(argv, "experimental-features = nix-command flakes\n")
        raise AssertionError("remedy must not run when nix.conf is already set")

    catalog.ensure_nix_conf(Device("h", backend=_Exec(respond)))


def test_nix_conf_writes_when_absent() -> None:
    state = {"conf": ""}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "cat /etc/nix/nix.conf" in cmd:
            return _cp(argv, state["conf"])
        if "printf" in cmd:
            state["conf"] = "experimental-features = nix-command flakes\n"
        return _cp(argv)

    catalog.ensure_nix_conf(Device("h", backend=_Exec(respond)))
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
    dev = Device("h", backend=_Exec(_routes({"df -k": _DF})))
    catalog.ensure_free_space(dev, "/mnt/data", 1_000_000)  # 1.6 GB free


def test_free_space_aborts_when_insufficient() -> None:
    dev = Device("h", backend=_Exec(_routes({"df -k": _DF})))
    with pytest.raises(Abort, match="only"):
        catalog.ensure_free_space(dev, "/mnt/data", 5_000_000_000)


# ── upload_firmware ───────────────────────────────────────────────────────────


def test_upload_streams_when_absent(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"du -b": ""}))  # absent → -1 != size
    catalog.upload_firmware(Device("h", backend=backend), image)
    assert backend.streams  # the firmware was uploaded


def test_upload_skips_when_already_uploaded(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"du -b": str(image.size)}))
    catalog.upload_firmware(Device("h", backend=backend), image)
    assert not backend.streams  # skipped


# ── sysupgrade ────────────────────────────────────────────────────────────────


def test_sysupgrade_skips_when_already_on_target(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"cat /etc/bos_version": image.version}))
    catalog.sysupgrade(Device("h", backend=backend), image)
    assert not any("sysupgrade" in argv[-1] for argv in backend.runs)


def test_sysupgrade_runs_with_force(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"cat /etc/bos_version": "older-version"}))
    catalog.sysupgrade(Device("h", backend=backend), image, force=True, assume_yes=True)
    assert any("sysupgrade -F " in argv[-1] for argv in backend.runs)


def test_sysupgrade_runs_with_assume_yes(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"cat /etc/bos_version": "older-version"}))
    catalog.sysupgrade(Device("h", backend=backend), image, assume_yes=True)
    assert any("sysupgrade " in argv[-1] for argv in backend.runs)


def test_sysupgrade_aborts_when_declined(tmp_path: Path) -> None:
    # No --yes, no --dry-run, and stdin is not a TTY under pytest, so
    # console.confirm returns False — the flash must be refused, not run.
    image = _image(tmp_path)
    backend = _Exec(_routes({"cat /etc/bos_version": "older-version"}))
    with pytest.raises(Abort, match="flash declined"):
        catalog.sysupgrade(Device("h", backend=backend), image)
    assert not any("sysupgrade " in argv[-1] for argv in backend.runs)


def test_sysupgrade_proceeds_under_dry_run(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"cat /etc/bos_version": "older-version"}))
    token = dry_run.set(True)
    try:
        catalog.sysupgrade(Device("h", backend=backend), image)
    finally:
        dry_run.reset(token)
    # dry-run logs the mutation instead of running it, so no real sysupgrade ssh.
    assert not any("sysupgrade " in argv[-1] for argv in backend.runs)


# ── wait_for_device ───────────────────────────────────────────────────────────


def test_wait_for_device_ok() -> None:
    catalog.wait_for_device(Device("h", backend=_Exec(_routes({}))), timeout=0)


def test_wait_for_device_times_out() -> None:
    with pytest.raises(Abort, match="did not return"):
        catalog.wait_for_device(Device("h", backend=_Exec(_unreachable)), timeout=0)

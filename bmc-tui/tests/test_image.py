"""Unit tests for the firmware Image tarball parsing."""

import io
import tarfile
from pathlib import Path
from typing import TYPE_CHECKING

from bmc_tui.image import Image

if TYPE_CHECKING:
    import pytest

_TOP = "sysupgrade-stm32mp15_ii3-emmc"


def _make_tar(
    path: Path,
    *,
    top: str = _TOP,
    command: bytes = b'UPGRADE_FW_VERSION="2026-06-14-x"\n',
    extra: tuple[str, ...] = ("rootfs.img",),
) -> None:
    with tarfile.open(path, "w") as tar:
        members = {"COMMAND": command, **{name: b"x" for name in extra}}
        for name, data in members.items():
            info = tarfile.TarInfo(f"{top}/{name}")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))


def test_size_and_remote_path(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    _make_tar(fw)
    image = Image(fw)
    assert image.size == fw.stat().st_size
    assert image.remote_path == "/tmp/fw.tar"


def test_sysupgrade_dir_and_is_sysupgrade(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    _make_tar(fw)
    image = Image(fw)
    assert image.sysupgrade_dir == _TOP
    assert image.is_sysupgrade is True


def test_rootfs_size_reads_the_member_size(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    _make_tar(fw)
    assert Image(fw).rootfs_size == 1  # the helper writes b"x" as rootfs.img


def test_version_parsed_from_command(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    _make_tar(fw, command=b'UPGRADE_FW_MAJOR="a"\nUPGRADE_FW_VERSION="2026-06-14-x"\n')
    assert Image(fw).version == "2026-06-14-x"


def test_not_sysupgrade_without_rootfs(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    _make_tar(fw, extra=())
    assert Image(fw).is_sysupgrade is False


def test_no_sysupgrade_dir_for_unrelated_tar(tmp_path: Path) -> None:
    fw = tmp_path / "fw.tar"
    _make_tar(fw, top="some-other-dir")
    assert Image(fw).sysupgrade_dir is None
    assert Image(fw).is_sysupgrade is False


def test_members_empty_for_non_tar(tmp_path: Path) -> None:
    junk = tmp_path / "junk.tar"
    junk.write_bytes(b"not a tar at all")
    assert Image(junk).members() == []
    assert Image(junk).is_sysupgrade is False


def test_print_shows_name_and_size(tmp_path: Path, capsys: "pytest.CaptureFixture[str]") -> None:
    fw = tmp_path / "fw.tar"
    _make_tar(fw)
    Image(fw).print()
    out = capsys.readouterr().out
    assert "fw.tar" in out
    assert "B" in out  # a size unit (B / KiB / MiB / …)

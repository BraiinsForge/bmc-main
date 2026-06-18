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
from bmc_tui.nix import Built, Pkg
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


# ── ensure_memory ─────────────────────────────────────────────────────────────


def test_memory_ok() -> None:
    dev = Device("h", backend=_Exec(_routes({"MemAvailable": "102924"})))  # ~100 MiB
    catalog.ensure_memory(dev, 50_000_000)


def test_memory_aborts_when_insufficient() -> None:
    dev = Device("h", backend=_Exec(_routes({"MemAvailable": "102924"})))
    with pytest.raises(Abort, match="free RAM"):
        catalog.ensure_memory(dev, 200_000_000)


# ── upload_firmware ───────────────────────────────────────────────────────────


def test_upload_streams_then_verifies_when_absent(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_cp)  # placeholder responder; the real one is set below

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1] if argv and argv[0] == "ssh" else " ".join(argv)
        if "sha256sum" in cmd:  # absent until the upload streams, then it matches
            return _cp(argv, image.sha256 if backend.streams else "")
        return _cp(argv)

    backend._respond = respond
    catalog.upload_firmware(Device("h", backend=backend), image)
    assert backend.streams  # the firmware was uploaded and its checksum verified


def test_upload_skips_when_already_uploaded(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"sha256sum": image.sha256}))  # present and intact
    catalog.upload_firmware(Device("h", backend=backend), image)
    assert not backend.streams  # skipped


def test_upload_aborts_on_checksum_mismatch(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"sha256sum": "deadbeef"}))  # device bytes never match
    with pytest.raises(Abort, match="checksum mismatch"):
        catalog.upload_firmware(Device("h", backend=backend), image)
    assert backend.streams  # streamed, then caught the bad upload before flashing


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


# ── nix package deploy ────────────────────────────────────────────────────────


class _Nix:
    """Fake Nix: resolve names from the attr leaf, build to a fake store path."""

    def __init__(self, widgets: tuple[str, ...] = (), out_dir: str = "") -> None:
        self.widgets = list(widgets)
        self.built: list[Pkg] = []
        self.copied: list[tuple[list[str], str]] = []
        self.out_dir = out_dir

    def discover_widgets(self) -> list[str]:
        return list(self.widgets)

    def build_out(self, attr: str) -> str:
        return self.out_dir

    def resolve(self, attr: str) -> Pkg:
        name = attr.rsplit(".", 1)[-1]
        return Pkg(name=name, version="1.0.0", installable=f"{attr}.pkg^out")

    def build(self, pkgs: list[Pkg]) -> list[Built]:
        self.built.extend(pkgs)
        return [
            Built(pkg.name, pkg.version, pkg.installable, store_path=f"/nix/store/{pkg.name}")
            for pkg in pkgs
        ]

    def copy(self, store_paths: list[str], dest: str) -> None:
        self.copied.append((store_paths, dest))


def test_resolve_discovers_core_plus_widgets() -> None:
    plan = catalog.Deployment(attrs=[])
    catalog.resolve_packages(_Nix(widgets=("clock", "weather")), plan)
    assert [p.name for p in plan.resolved] == ["core", "clock", "weather"]
    assert plan.attrs[0] == ".#deck-packages.core"


def test_resolve_uses_explicit_packages() -> None:
    plan = catalog.Deployment(attrs=[".#deck-packages.core"])
    catalog.resolve_packages(_Nix(widgets=("clock",)), plan)  # widgets ignored when explicit
    assert [p.name for p in plan.resolved] == ["core"]


def test_build_realises_each_resolved() -> None:
    plan = catalog.Deployment(attrs=[], resolved=[Pkg("core", "1.0", ".#x.pkg^out")])
    catalog.build_packages(_Nix(), plan)
    assert [b.store_path for b in plan.built] == ["/nix/store/core"]


def test_copy_closures_sends_built_paths() -> None:
    nix = _Nix()
    dev = Device("h", backend=_Exec(_routes({})))
    plan = catalog.Deployment(
        attrs=[], built=[Built("core", "1.0", ".#x.pkg^out", "/nix/store/core")]
    )
    catalog.copy_closures(nix, dev, plan)
    assert nix.copied == [(["/nix/store/core"], dev.copy_dest)]


def test_register_packages_builds_cli_command() -> None:
    backend = _Exec(_routes({}))
    dev = Device("h", backend=backend)
    plan = catalog.Deployment(
        attrs=[], built=[Built("core", "1.0", ".#x.pkg^out", "/nix/store/core")]
    )
    catalog.register_packages(dev, plan)
    cmd = backend.runs[-1][-1]
    assert "bmc-nix-cli add-packages" in cmd
    assert "--name core --version 1.0 --store-path /nix/store/core" in cmd


def test_generation_number_parses_link_path() -> None:
    assert catalog._generation_number("/nix/var/nix/gcroots/profiles/bmc/3-link") == "3"


def test_generation_number_absent_or_unexpected() -> None:
    # None under dry-run, "" when nothing was printed, non-`-link` stdout ignored.
    assert catalog._generation_number(None) is None
    assert catalog._generation_number("") is None
    assert catalog._generation_number("Profile unchanged.") is None


def test_ensure_nix_cli_skips_when_present() -> None:
    nix = _Nix()
    dev = Device("h", backend=_Exec(_routes({"test -x": "ok"})))
    catalog.ensure_nix_cli(nix, dev)
    assert not nix.built  # no bootstrap needed


def test_ensure_nix_cli_bootstraps_when_absent() -> None:
    state = {"present": False}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "add-packages" in cmd:
            state["present"] = True
        if "test -x" in cmd:
            return _cp(argv, "ok" if state["present"] else "")
        return _cp(argv)

    nix = _Nix()
    catalog.ensure_nix_cli(nix, Device("h", backend=_Exec(respond)))
    assert [p.name for p in nix.built] == ["bmc-nix-cli"]
    assert nix.copied  # the closure was shipped


# ── device init ───────────────────────────────────────────────────────────────


def test_store_absent_passes_when_clean() -> None:
    # Probe returns nothing — neither store dir exists or both are empty.
    catalog.ensure_store_absent(Device("h", backend=_Exec(_routes({}))))


def test_store_absent_aborts_when_populated() -> None:
    backend = _Exec(_routes({"for d in": "core\nbmc-nix-cli"}))
    with pytest.raises(Abort, match="already populated"):
        catalog.ensure_store_absent(Device("h", backend=backend))


def test_mount_nix_store_skips_when_already_mounted() -> None:
    backend = _Exec(_routes({"/proc/mounts": "/mnt/data/nix /nix none bind 0 0"}))
    catalog.mount_nix_store(Device("h", backend=backend))
    assert not any("mount --bind" in argv[-1] for argv in backend.runs)


def test_mount_nix_store_binds_when_absent() -> None:
    backend = _Exec(_routes({}))  # /proc/mounts probe is empty → not mounted
    catalog.mount_nix_store(Device("h", backend=backend))
    assert any("mount --bind /mnt/data/nix /nix" in argv[-1] for argv in backend.runs)


def test_build_init_tarball_locates_the_archive(tmp_path: Path) -> None:
    (tmp_path / "nix-2026.tar.gz").write_bytes(b"x")
    plan = catalog.Provisioning()
    catalog.build_init_tarball(_Nix(out_dir=str(tmp_path)), plan)
    assert plan.tarball == tmp_path / "nix-2026.tar.gz"


def test_build_init_tarball_aborts_when_archive_missing(tmp_path: Path) -> None:
    plan = catalog.Provisioning()
    with pytest.raises(Abort, match=r"expected one \.tar\.gz"):
        catalog.build_init_tarball(_Nix(out_dir=str(tmp_path)), plan)


def test_stream_init_tarball_streams_into_tar(tmp_path: Path) -> None:
    tarball = tmp_path / "nix.tar.gz"
    tarball.write_bytes(b"init-bytes")
    backend = _Exec(_routes({}))
    catalog.stream_init_tarball(Device("h", backend=backend), catalog.Provisioning(tarball))
    argv, data = backend.streams[0]
    assert argv[-1] == "tar xzf - -C /"
    assert data == b"init-bytes"


def test_stream_init_tarball_skips_under_dry_run(tmp_path: Path) -> None:
    tarball = tmp_path / "nix.tar.gz"
    tarball.write_bytes(b"x")
    backend = _Exec(_routes({}))
    token = dry_run.set(True)
    try:
        catalog.stream_init_tarball(Device("h", backend=backend), catalog.Provisioning(tarball))
    finally:
        dry_run.reset(token)
    assert backend.streams == []


def test_stream_init_tarball_raises_without_built_tarball() -> None:
    with pytest.raises(RuntimeError, match="BUG"):
        catalog.stream_init_tarball(Device("h", backend=_Exec(_routes({}))), catalog.Provisioning())


def test_activate_profile_runs_the_entrypoint() -> None:
    backend = _Exec(_routes({}))
    catalog.activate_profile(Device("h", backend=backend))
    assert backend.runs[-1][-1].endswith("/bmc/1-link/core/activation/entrypoint")

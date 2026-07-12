"""Unit tests for the deploy stage catalog."""

import io
import json
import shlex
import subprocess
import sys
import tarfile
from collections.abc import Callable, Iterable
from dataclasses import replace
from pathlib import Path
from typing import Required, TypedDict, Unpack

import pytest

from bmc_tui import catalog, rig
from bmc_tui.device import Device
from bmc_tui.image import Image
from bmc_tui.nix import Built, Pkg
from bmc_tui.procedures.init import Init
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


def _image(
    tmp_path: Path,
    *,
    top: str = _TOP,
    extra: tuple[str, ...] = ("rootfs.img",),
    name: str = "fw.tar",
    version: str = "2026-06-14-x",
) -> Image:
    fw = tmp_path / name
    with tarfile.open(fw, "w") as tar:
        command = f'UPGRADE_FW_VERSION="{version}"\n'.encode()
        files = {"COMMAND": command, **{n: b"x" for n in extra}}
        for name_, data in files.items():
            info = tarfile.TarInfo(f"{top}/{name_}")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    return Image(fw)


_TARBALL_ATTR = ".#init-tarball-armv7"
_CLI_ATTR = ".#bmc-nix-cli-armv7-release"


class _FakeNix:
    """Attribute-aware fake Nix backend: build_out maps flake attrs to
    prepared output directories. Implements the full Nix protocol so ty
    accepts it wherever a Nix is annotated; the package-oriented methods
    are unused by the init stages."""

    def __init__(self, outs: dict[str, Path]) -> None:
        self._outs = outs
        self.build_file_calls: list[tuple[str, list[str], dict[str, str]]] = []

    def discover_widgets(self) -> list[str]:
        return []

    def list_packages(self) -> list[str]:
        return []

    def resolve(self, attr: str) -> Pkg:
        raise NotImplementedError

    def build(self, pkgs: list[Pkg]) -> list[Built]:
        return []

    def build_out(self, attr: str) -> str:
        return str(self._outs[attr])

    def build_file(self, file: str, attrs: list[str], args: dict[str, str]) -> list[str]:
        self.build_file_calls.append((file, attrs, args))
        return [str(self._outs[attr]) for attr in attrs]

    def generate_cache_key(self, name: str, secret: Path) -> str:
        secret.write_text("sk")
        return f"{name}:PUBLICKEY"

    def copy_signed(self, store_paths: list[str], cache: Path, secret: Path) -> None:
        cache.mkdir(parents=True, exist_ok=True)
        (cache / "fake.narinfo").write_text("StorePath: /nix/store/fake\n")

    def copy(self, store_paths: list[str], dest: str) -> None:
        return None


def _cli_out(tmp_path: Path) -> Path:
    out = tmp_path / "cli-out"
    (out / "bin").mkdir(parents=True, exist_ok=True)
    (out / "bin" / "bmc-nix-cli").write_bytes(b"elf")
    return out


def _fake_nix(tmp_path: Path, tarball_out: Path | None = None) -> _FakeNix:
    tarball = tarball_out if tarball_out is not None else _tarball_out(tmp_path)
    return _FakeNix({_TARBALL_ATTR: tarball, _CLI_ATTR: _cli_out(tmp_path)})


def _tarball_out(
    tmp_path: Path,
    *,
    overrides: dict[str, str] | None = None,
    write_archive: bool = True,
    meta: bool = True,
) -> Path:
    out = tmp_path / "tarball-out"
    out.mkdir(parents=True, exist_ok=True)
    payload = {
        "bos_version": "2026-06-14-x",
        "profile_path": "/nix/var/nix/gcroots/profiles/bmc",
        "tarball_name": "nix-2026-06-14-x.tar.gz",
        **(overrides or {}),
    }
    if meta:
        (out / "metadata.json").write_text(json.dumps(payload))
    if write_archive and "/" not in payload["tarball_name"]:
        (out / payload["tarball_name"]).write_bytes(b"tarball-bytes")
    return out


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

    def __init__(
        self, widgets: tuple[str, ...] = (), out_dir: str = "", packages: tuple[str, ...] = ()
    ) -> None:
        self.widgets = list(widgets)
        self.packages = list(packages) or ["core", *self.widgets]
        self.built: list[Pkg] = []
        self.copied: list[tuple[list[str], str]] = []
        self.out_dir = out_dir

    def discover_widgets(self) -> list[str]:
        return list(self.widgets)

    def list_packages(self) -> list[str]:
        return list(self.packages)

    def build_out(self, attr: str) -> str:
        return self.out_dir

    def build_file(self, file: str, attrs: list[str], args: dict[str, str]) -> list[str]:
        raise NotImplementedError

    def generate_cache_key(self, name: str, secret: Path) -> str:
        raise NotImplementedError

    def copy_signed(self, store_paths: list[str], cache: Path, secret: Path) -> None:
        raise NotImplementedError

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
    assert [p.name for p in plan.resolved] == ["core", "bmc-nix-cli", "clock", "weather"]
    assert plan.attrs[0] == ".#deck-packages.core"


def test_resolve_uses_explicit_packages() -> None:
    plan = catalog.Deployment(attrs=[".#deck-packages.core"])
    catalog.resolve_packages(_Nix(widgets=("clock",)), plan)  # widgets ignored when explicit
    assert [p.name for p in plan.resolved] == ["core"]


def test_resolve_aborts_with_suggestion_on_unknown_package() -> None:
    class _BadNix(_Nix):
        def resolve(self, attr: str) -> Pkg:
            raise subprocess.CalledProcessError(1, ["nix", "eval", attr])

    plan = catalog.Deployment(attrs=[".#deck-packages.image"])
    with pytest.raises(Abort, match="widget-image"):
        catalog.resolve_packages(_BadNix(widgets=("widget-image", "widget-clock")), plan)


def test_resolve_suggests_non_widget_packages() -> None:
    class _BadNix(_Nix):
        def resolve(self, attr: str) -> Pkg:
            raise subprocess.CalledProcessError(1, ["nix", "eval", attr])

    plan = catalog.Deployment(attrs=[".#deck-packages.frontend"])
    nix = _BadNix(packages=("core", "bmc-frontend", "widget-image"))
    with pytest.raises(Abort, match="bmc-frontend"):
        catalog.resolve_packages(nix, plan)


def test_resolve_qualifies_bare_package_names() -> None:
    plan = catalog.Deployment(attrs=["core", "widget-image"])
    catalog.resolve_packages(_Nix(), plan)
    assert plan.attrs == [".#deck-packages.core", ".#deck-packages.widget-image"]


def test_resolve_leaves_qualified_attrs_alone() -> None:
    plan = catalog.Deployment(attrs=[".#armv7-nixpkgs.strace"])
    catalog.resolve_packages(_Nix(), plan)
    assert plan.attrs == [".#armv7-nixpkgs.strace"]


def test_package_prefix_maps_profile_to_attr_root() -> None:
    assert catalog.package_prefix("release") == ".#deck-packages"
    assert catalog.package_prefix("debug") == ".#deck-packages-debug"


def test_resolve_uses_debug_prefix_for_discovery_and_bare_names() -> None:
    plan = catalog.Deployment(attrs=[], prefix=catalog.package_prefix("debug"))
    catalog.resolve_packages(_Nix(widgets=("clock",)), plan)
    assert plan.attrs == [
        ".#deck-packages-debug.core",
        ".#deck-packages-debug.bmc-nix-cli",
        ".#deck-packages-debug.clock",
    ]


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


def _flip_clock_plan() -> catalog.Deployment:
    built = Built("widget-flip-clock", "1.0", ".#x^out", store_path="/nix/store/wfc")
    return catalog.Deployment(attrs=[], built=[built])


def test_remove_legacy_flip_clock_issues_remove_command() -> None:
    backend = _Exec(_routes({}))
    catalog.remove_legacy_flip_clock(Device("h", backend=backend), _flip_clock_plan())
    assert "remove-packages --name flip-clock" in backend.runs[-1][-1]


def test_remove_legacy_flip_clock_skips_without_successor() -> None:
    # No widget-flip-clock in the deploy → no conflict → nothing to remove.
    backend = _Exec(_routes({}))
    plan = catalog.Deployment(
        attrs=[], built=[Built("core", "1.0", ".#x^out", store_path="/nix/store/core")]
    )
    catalog.remove_legacy_flip_clock(Device("h", backend=backend), plan)
    assert backend.runs == []


def test_remove_legacy_flip_clock_tolerates_failure() -> None:
    # A device that never had the legacy package makes remove-packages exit
    # non-zero; the deploy must swallow it rather than abort.
    catalog.remove_legacy_flip_clock(Device("h", backend=_Exec(_unreachable)), _flip_clock_plan())


def test_restart_compositor_runs_on_confirm(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("bmc_tui.console.confirm", lambda _q: True)
    backend = _Exec(_routes({}))
    catalog.restart_compositor(Device("h", backend=backend))
    assert "/etc/init.d/bmc-compositor restart" in backend.runs[-1][-1]


def test_restart_compositor_skips_on_decline(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("bmc_tui.console.confirm", lambda _q: False)
    backend = _Exec(_routes({}))
    catalog.restart_compositor(Device("h", backend=backend))
    assert backend.runs == []


def test_restart_compositor_skips_under_dry_run(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("bmc_tui.console.confirm", lambda _q: True)
    token = dry_run.set(True)
    backend = _Exec(_routes({}))
    try:
        catalog.restart_compositor(Device("h", backend=backend))
    finally:
        dry_run.reset(token)
    assert backend.runs == []


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


# ── build_init_tarball metadata validation ────────────────────────────────────


def test_build_tarball_reads_metadata(tmp_path: Path) -> None:
    plan = catalog.Provisioning()
    catalog.build_init_tarball(_fake_nix(tmp_path), plan)
    assert plan.tarball is not None
    assert plan.tarball.name == "nix-2026-06-14-x.tar.gz"
    assert plan.profile_path == "/nix/var/nix/gcroots/profiles/bmc"


def test_build_tarball_aborts_without_metadata(tmp_path: Path) -> None:
    nix = _fake_nix(tmp_path, _tarball_out(tmp_path, meta=False))
    with pytest.raises(Abort, match=r"metadata\.json"):
        catalog.build_init_tarball(nix, catalog.Provisioning())


def test_build_tarball_aborts_on_missing_field(tmp_path: Path) -> None:
    nix = _fake_nix(tmp_path, _tarball_out(tmp_path, overrides={"bos_version": ""}))
    with pytest.raises(Abort, match="bos_version"):
        catalog.build_init_tarball(nix, catalog.Provisioning())


def test_build_tarball_aborts_on_non_basename(tmp_path: Path) -> None:
    nix = _fake_nix(tmp_path, _tarball_out(tmp_path, overrides={"tarball_name": "../evil.tar.gz"}))
    with pytest.raises(Abort, match="basename"):
        catalog.build_init_tarball(nix, catalog.Provisioning())


def test_build_tarball_aborts_on_missing_archive(tmp_path: Path) -> None:
    nix = _fake_nix(tmp_path, _tarball_out(tmp_path, write_archive=False))
    with pytest.raises(Abort, match="missing archive"):
        catalog.build_init_tarball(nix, catalog.Provisioning())


def test_build_tarball_aborts_on_relative_profile_path(tmp_path: Path) -> None:
    out = _tarball_out(tmp_path, overrides={"profile_path": "var/profiles/bmc"})
    nix = _fake_nix(tmp_path, out)
    with pytest.raises(Abort, match="under /nix"):
        catalog.build_init_tarball(nix, catalog.Provisioning())


def test_build_tarball_aborts_on_traversal_profile_path(tmp_path: Path) -> None:
    # startswith("/nix/") alone would let this escape to /tmp.
    out = _tarball_out(tmp_path, overrides={"profile_path": "/nix/../tmp/profile"})
    nix = _fake_nix(tmp_path, out)
    with pytest.raises(Abort, match="under /nix"):
        catalog.build_init_tarball(nix, catalog.Provisioning())


# ── ensure_store_absent ───────────────────────────────────────────────────────


def _store_state(
    *,
    mounted: bool = False,
    identical: bool = False,
    backing_exists: bool = False,
    backing_listing: str = "",
    rootfs_listing: str = "",
) -> _Respond:
    return _routes(
        {
            "/proc/mounts": "nix-line" if mounted else "",
            "-ef": "yes" if identical else "",
            "ls -A /mnt/data/nix": backing_listing,
            "[ -d /mnt/data/nix ] && echo yes": "yes" if backing_exists else "",
            "ls -A /nix": rootfs_listing,
        }
    )


def test_store_absent_passes_when_nothing_exists() -> None:
    dev = Device("h", backend=_Exec(_store_state()))
    catalog.ensure_store_absent(dev)


def test_store_absent_removes_empty_unmounted_backing_dir() -> None:
    exec_ = _Exec(_store_state(backing_exists=True))
    catalog.ensure_store_absent(Device("h", backend=exec_))
    joined = [" ".join(argv) for argv in exec_.runs]
    assert any("rmdir /mnt/data/nix" in c for c in joined)
    assert not any("umount" in c for c in joined)


def test_store_absent_unmounts_identical_bind_before_removal() -> None:
    exec_ = _Exec(_store_state(mounted=True, identical=True, backing_exists=True))
    catalog.ensure_store_absent(Device("h", backend=exec_))
    joined = [" ".join(argv) for argv in exec_.runs]
    umount = next(i for i, c in enumerate(joined) if "umount /nix" in c)
    rmdir = next(i for i, c in enumerate(joined) if "rmdir /mnt/data/nix" in c)
    assert umount < rmdir, "the bind mount must be released before its backing dir is removed"


def test_store_absent_refuses_populated_backing_dir() -> None:
    dev = Device("h", backend=_Exec(_store_state(backing_exists=True, backing_listing="store")))
    with pytest.raises(Abort, match="populated"):
        catalog.ensure_store_absent(dev)


def test_store_absent_refuses_foreign_mount_without_backing_dir() -> None:
    dev = Device("h", backend=_Exec(_store_state(mounted=True, identical=False)))
    with pytest.raises(Abort, match="foreign"):
        catalog.ensure_store_absent(dev)


def test_store_absent_refuses_rootfs_nix_content() -> None:
    dev = Device("h", backend=_Exec(_store_state(rootfs_listing="store")))
    with pytest.raises(Abort, match="rootfs"):
        catalog.ensure_store_absent(dev)


class _BindMountedEmptyStore:
    """Stateful responder: an empty /mnt/data/nix already bind-mounted at
    /nix. umount/rmdir/`cli mount`/`cli init` flip the tracked state;
    reads answer from it, so the whole procedure can run against it."""

    def __init__(self, profile_path: str, *, fail_umount: bool = False) -> None:
        self.profile_path = profile_path
        self.fail_umount = fail_umount
        self.mounted = True
        self.backing_exists = True

    def __call__(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":  # noqa: PLR0911  one return per probe reads clearest
        cmd = argv[-1] if argv and argv[0] == "ssh" else " ".join(argv)
        if "umount /nix" in cmd:
            if self.fail_umount:
                raise subprocess.CalledProcessError(32, argv)
            self.mounted = False
            return _cp(argv)
        if "rmdir /mnt/data/nix" in cmd:
            self.backing_exists = False
            return _cp(argv)
        if "bmc-nix-cli" in cmd and cmd.rstrip().endswith(" mount"):
            self.mounted = True
            self.backing_exists = True
            return _cp(argv)
        if " init " in cmd and "--tarball" in cmd:
            self.backing_exists = True
            return _cp(argv, self.profile_path)
        if "grep ' /nix ' /proc/mounts" in cmd:
            return _cp(argv, "nix-line" if self.mounted else "")
        if "grep ' /mnt/data ' /proc/mounts" in cmd:
            return _cp(argv, "data-line")
        if "-ef" in cmd:
            return _cp(argv, "yes" if self.mounted else "")
        if "ls -A /mnt/data/nix" in cmd:
            return _cp(argv, "")
        if "[ -d /mnt/data/nix ] && echo yes" in cmd:
            return _cp(argv, "yes" if self.backing_exists else "")
        return _cp(argv)


def test_store_absent_aborts_when_umount_fails() -> None:
    responder = _BindMountedEmptyStore("/nix/var/nix/gcroots/profiles/bmc", fail_umount=True)
    dev = Device("h", backend=_Exec(responder))
    with pytest.raises(subprocess.CalledProcessError):
        catalog.ensure_store_absent(dev)


# ── run_cli_init / activate_profile / cleanup ────────────────────────────────


def _pushed_plan(tmp_path: Path) -> catalog.Provisioning:
    plan = catalog.Provisioning()
    plan.profile_path = "/nix/var/nix/gcroots/profiles/bmc"
    plan.remote_tarball = "/mnt/data/nix-x.tar.gz.deck-init.1"
    plan.tarball = tmp_path / "t.tar.gz"
    return plan


def test_run_cli_init_accepts_exact_profile_path(tmp_path: Path) -> None:
    plan = _pushed_plan(tmp_path)
    dev = Device("h", backend=_Exec(_routes({" init ": plan.profile_path or ""})))
    catalog.run_cli_init(dev, plan)


def test_run_cli_init_aborts_on_stdout_mismatch(tmp_path: Path) -> None:
    plan = _pushed_plan(tmp_path)
    dev = Device("h", backend=_Exec(_routes({" init ": "/nix/somewhere/else"})))
    with pytest.raises(Abort, match="expected"):
        catalog.run_cli_init(dev, plan)


def test_activate_aborts_when_mount_identity_fails(tmp_path: Path) -> None:
    plan = _pushed_plan(tmp_path)
    dev = Device("h", backend=_Exec(_routes({"-ef": ""})))
    with pytest.raises(Abort, match="not backed by"):
        catalog.activate_profile(dev, plan)


def test_cleanup_removes_pushed_files(tmp_path: Path) -> None:
    plan = _pushed_plan(tmp_path)
    exec_ = _Exec(_routes({}))
    catalog.cleanup_remote_artifacts(Device("h", backend=exec_), plan)
    joined = [" ".join(argv) for argv in exec_.runs]
    assert any("rm -f" in c and "/mnt/data/nix-x.tar.gz.deck-init.1" in c for c in joined)


# ── init procedure ────────────────────────────────────────────────────────────


def test_init_procedure_dry_run_asserts_nothing(tmp_path: Path) -> None:
    exec_ = _Exec(_store_state())
    token = dry_run.set(True)
    try:
        Init(device="h", dry_run=True).run(
            dev=Device("h", backend=exec_),
            backend=_fake_nix(tmp_path),
        )
    finally:
        dry_run.reset(token)
    assert exec_.streams == [], "dry-run must not push anything"


def test_init_procedure_succeeds_over_identical_empty_bind(tmp_path: Path) -> None:
    responder = _BindMountedEmptyStore("/nix/var/nix/gcroots/profiles/bmc")
    exec_ = _Exec(responder)
    Init(device="h").run(dev=Device("h", backend=exec_), backend=_fake_nix(tmp_path))
    joined = [" ".join(argv) for argv in exec_.runs]
    umount = next(i for i, c in enumerate(joined) if "umount /nix" in c)
    rmdir = next(i for i, c in enumerate(joined) if "rmdir /mnt/data/nix" in c)
    assert umount < rmdir, "the stale bind must be released before its backing dir is removed"
    assert any("entrypoint" in c for c in joined), "activation must run after the re-init"


def test_init_procedure_cleans_up_on_stage_failure(tmp_path: Path) -> None:
    # A populated backing store makes ensure_store_absent abort after the
    # CLI push; cleanup must still remove the pushed CLI.
    exec_ = _Exec(_store_state(backing_exists=True, backing_listing="store"))
    with pytest.raises(Abort):
        Init(device="h").run(
            dev=Device("h", backend=exec_),
            backend=_fake_nix(tmp_path),
        )
    joined = [" ".join(argv) for argv in exec_.runs]
    assert any("rm -f" in c and "bmc-nix-cli.deck-init" in c for c in joined)


def test_init_procedure_cleans_up_pushed_tarball_on_init_failure(tmp_path: Path) -> None:
    # init printing the wrong path aborts run_cli_init AFTER the tarball
    # push; cleanup must remove the pushed tarball too, so the remote
    # path is recorded before the (possibly failing) upload starts.
    exec_ = _Exec(_routes({" init ": "/nix/somewhere/else"}))
    with pytest.raises(Abort):
        Init(device="h").run(
            dev=Device("h", backend=exec_),
            backend=_fake_nix(tmp_path),
        )
    joined = [" ".join(argv) for argv in exec_.runs]
    cleanup = next(c for c in joined if "rm -f" in c)
    assert "bmc-nix-cli.deck-init" in cleanup
    assert ".tar.gz.deck-init." in cleanup


def test_init_procedure_cleans_up_when_tarball_push_fails(tmp_path: Path) -> None:
    # The upload itself dying must still leave cleanup with both remote
    # paths — this is what pins remote_tarball being recorded BEFORE the
    # stream starts (a record-after-push regression makes it fail).
    class _TarballPushExplodes(_Exec):
        def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
            if any(".tar.gz.deck-init." in part for part in argv):
                raise subprocess.CalledProcessError(1, argv)
            super().stream(argv, chunks)

    exec_ = _TarballPushExplodes(_routes({}))
    # Stages don't catch subprocess errors; the raw failure propagates,
    # but Init.run's finally must still sweep both pushed files.
    with pytest.raises(subprocess.CalledProcessError):
        Init(device="h").run(
            dev=Device("h", backend=exec_),
            backend=_fake_nix(tmp_path),
        )
    joined = [" ".join(argv) for argv in exec_.runs]
    cleanup = next(c for c in joined if "rm -f" in c)
    assert "bmc-nix-cli.deck-init" in cleanup
    assert ".tar.gz.deck-init." in cleanup


# ── e2e upgrade cycle ─────────────────────────────────────────────────────────


def _cycle(
    *,
    generation_before: int | None = None,
    installed_before: set[str] | None = None,
) -> catalog.UpgradeCycle:
    cycle = catalog.UpgradeCycle(
        password="", port=8080, index_port=8081, key_dir=Path("/nonexistent")
    )
    cycle.generation_before = generation_before
    cycle.installed_before = installed_before
    return cycle


def test_snapshot_profile_reads_current_generation() -> None:
    cycle = _cycle()
    manifest = _manifest(core="/nix/store/abc-core")
    backend = _Exec(_routes({"readlink": "5-link", "cat": manifest}))
    catalog.snapshot_profile(Device("h", backend=backend), cycle)
    assert cycle.generation_before == 5
    assert cycle.installed_before == {"core"}


def test_snapshot_profile_aborts_on_unexpected_link() -> None:
    backend = _Exec(_routes({"readlink": "garbage"}))
    with pytest.raises(Abort, match="unexpected current generation link"):
        catalog.snapshot_profile(Device("h", backend=backend), _cycle())


def _manifest(**paths: str) -> str:
    return json.dumps({"packages": {name: {"store_path": p} for name, p in paths.items()}})


def test_verify_profile_advanced_ok() -> None:
    cycle = _cycle(generation_before=5, installed_before={"core"})
    built = Built("core", "1.0", ".#x", store_path="/nix/store/abc-core")
    manifest = _manifest(core="/nix/store/abc-core")
    backend = _Exec(_routes({"readlink": "6-link", "cat": manifest}))
    plan = catalog.Deployment(attrs=[], built=[built])
    catalog.verify_profile_advanced(Device("h", backend=backend), plan, cycle)


def test_verify_profile_advanced_aborts_when_generation_unchanged() -> None:
    cycle = _cycle(generation_before=5, installed_before=set())
    backend = _Exec(_routes({"readlink": "5-link"}))
    with pytest.raises(Abort, match="still 5"):
        catalog.verify_profile_advanced(
            Device("h", backend=backend), catalog.Deployment(attrs=[]), cycle
        )


def test_verify_profile_advanced_aborts_on_missing_store_path() -> None:
    cycle = _cycle(generation_before=5, installed_before={"core"})
    built = Built("core", "1.0", ".#x", store_path="/nix/store/abc-core")
    manifest = _manifest(core="/nix/store/other")
    backend = _Exec(_routes({"readlink": "6-link", "cat": manifest}))
    plan = catalog.Deployment(attrs=[], built=[built])
    with pytest.raises(Abort, match="not replaced by their served store paths"):
        catalog.verify_profile_advanced(Device("h", backend=backend), plan, cycle)


def test_verify_profile_advanced_aborts_on_path_under_wrong_package() -> None:
    cycle = _cycle(generation_before=5, installed_before={"core"})
    built = Built("core", "1.0", ".#x", store_path="/nix/store/abc-core")
    # The served path is present, but attached to a different package — a
    # substring check would have wrongly passed.
    manifest = _manifest(other="/nix/store/abc-core", core="/nix/store/stale-core")
    backend = _Exec(_routes({"readlink": "6-link", "cat": manifest}))
    plan = catalog.Deployment(attrs=[], built=[built])
    with pytest.raises(Abort, match="not replaced by their served store paths"):
        catalog.verify_profile_advanced(Device("h", backend=backend), plan, cycle)


def test_verify_profile_advanced_aborts_on_non_json_manifest() -> None:
    cycle = _cycle(generation_before=5, installed_before={"core"})
    built = Built("core", "1.0", ".#x", store_path="/nix/store/abc-core")
    backend = _Exec(_routes({"readlink": "6-link", "cat": "/nix/store/abc-core"}))
    plan = catalog.Deployment(attrs=[], built=[built])
    with pytest.raises(Abort, match="not a package manifest"):
        catalog.verify_profile_advanced(Device("h", backend=backend), plan, cycle)


def test_upgrade_server_argv_serves_widgets_with_metadata() -> None:
    # widget-* packages must go in as --widget so the server attaches picker
    # metadata; other packages stay plain --package entries.
    argv = catalog._upgrade_server_argv(
        host="10.0.0.1",
        port=8080,
        index_port=8081,
        key_dir=Path("/k"),
        built=[
            Built("core", "1.0", ".#core^out", store_path="/nix/store/core"),
            Built("widget-weather", "0.1.0", ".#w^out", store_path="/nix/store/weather"),
        ],
    )
    assert argv[argv.index("--package") + 1] == "core=1.0=/nix/store/core"
    assert argv[argv.index("--widget") + 1] == "widget-weather=0.1.0=/nix/store/weather"


def test_stop_upgrade_server_is_a_noop_without_a_server() -> None:
    catalog.stop_upgrade_server(_cycle())


def test_stream_events_decodes_back_to_back_messages() -> None:
    script = (
        "import sys;"
        'sys.stdout.write(\'{\\n  "packagePhase": "PACKAGE_UPGRADE_PHASE_REALIZING"\\n}\\n\');'
        "sys.stdout.write('{\\n  \"finished\": {}\\n}\\n')"
    )
    events = catalog._stream_events([sys.executable, "-c", script])
    assert events == [
        {"packagePhase": "PACKAGE_UPGRADE_PHASE_REALIZING"},
        {"finished": {}},
    ]


def test_stream_events_aborts_on_stream_failure() -> None:
    script = "import sys; sys.stderr.write('boom'); sys.exit(1)"
    with pytest.raises(Abort, match="boom"):
        catalog._stream_events([sys.executable, "-c", script])


# ── install e2e parsers ───────────────────────────────────────────────────────


def test_installable_widget_names_extracts_package_names() -> None:
    response = {
        "widgets": [
            {"packageName": "widget-weather", "uid": "u1", "displayName": "Weather"},
            {"packageName": "widget-ticker", "uid": "u2", "displayName": "Ticker"},
        ]
    }
    assert catalog._installable_widget_names(response) == ["widget-weather", "widget-ticker"]


def test_installed_package_names_from_list_packages_json() -> None:
    payload = {
        "packages": [
            {"name": "core", "version": "2.0.0"},
            {"name": "widget-weather", "version": "1.3.0"},
        ]
    }
    assert catalog._installed_package_names(payload) == ["core", "widget-weather"]


def _list_packages(*names: str) -> str:
    return json.dumps({"packages": [{"name": name} for name in names]})


def test_remove_package_removes_when_present() -> None:
    backend = _Exec(_routes({"list-packages": _list_packages("core", "widget-blockheight")}))
    catalog.remove_package(Device("h", backend=backend), _cycle(), "widget-blockheight")
    assert any("remove-packages --name widget-blockheight" in run[-1] for run in backend.runs)


def test_remove_package_skips_when_absent() -> None:
    # A prior run that removed the widget but aborted before reinstalling leaves it
    # absent; the re-run must skip rather than error on remove-packages.
    backend = _Exec(_routes({"list-packages": _list_packages("core")}))
    catalog.remove_package(Device("h", backend=backend), _cycle(), "widget-blockheight")
    assert all("remove-packages" not in run[-1] for run in backend.runs)


# ── sysupgrade e2e ────────────────────────────────────────────────────────────

_NIX_ERA_EXTRA = ("rootfs.img", "bmc-nix-cli", "servers.json.default")


def _e2e_image(tmp_path: Path, *, name: str, version: str) -> Image:
    return _image(tmp_path, name=name, version=version, extra=_NIX_ERA_EXTRA)


def _index_out(tmp_path: Path, version: str, packages: list[tuple[str, str]]) -> Path:
    out = tmp_path / f"index-{version}"
    out.mkdir(parents=True, exist_ok=True)
    entries = [{"name": n, "version": "1.0", "store_path": p} for n, p in packages]
    (out / rig.INDEX_NAME).write_text(
        json.dumps({"version": 1, "indexes": [], "caches": [], "packages": entries})
    )
    return out


def _e2e_tarball_out(tmp_path: Path, version: str, *, meta_version: str | None = None) -> Path:
    return _tarball_out(
        tmp_path / f"tar-{version}",
        overrides={
            "bos_version": meta_version if meta_version is not None else version,
            "tarball_name": f"nix-{version}.tar.gz",
        },
    )


def _e2e_nix(tmp_path: Path, *, version_a: str = "va", version_b: str = "vb") -> _FakeNix:
    return _FakeNix(
        {
            "index-a": _index_out(tmp_path, version_a, [("bmc-nix-cli", "/nix/store/cli-a")]),
            "tarball-a": _e2e_tarball_out(tmp_path, version_a),
            "index-b": _index_out(tmp_path, version_b, [("bmc-nix-cli", "/nix/store/cli-b")]),
            "tarball-b": _e2e_tarball_out(tmp_path, version_b),
            _CLI_ATTR: _cli_out(tmp_path),
        }
    )


def _e2e_run(tmp_path: Path, *, version_a: str = "va", version_b: str = "vb") -> catalog.E2eRun:
    return catalog.E2eRun(
        image_a=_e2e_image(tmp_path, name="a.tar", version=version_a),
        image_b=_e2e_image(tmp_path, name="b.tar", version=version_b),
    )


def test_e2e_inputs_reject_equal_versions(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path, version_a="same", version_b="same")
    with pytest.raises(Abort, match="same firmware version"):
        catalog.validate_e2e_inputs(run)


def test_e2e_inputs_reject_a_legacy_payload(tmp_path: Path) -> None:
    run = catalog.E2eRun(
        image_a=_image(tmp_path, name="a.tar", version="va"),
        image_b=_e2e_image(tmp_path, name="b.tar", version="vb"),
    )
    with pytest.raises(Abort, match="bmc-nix-cli"):
        catalog.validate_e2e_inputs(run)


def test_e2e_inputs_accept_a_nix_era_pair(tmp_path: Path) -> None:
    catalog.validate_e2e_inputs(_e2e_run(tmp_path))


def test_build_e2e_artifacts_builds_all_attrs_in_one_call(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path)
    nix = _e2e_nix(tmp_path)
    catalog.build_e2e_artifacts(nix, run)
    assert nix.build_file_calls == [
        (
            "nix/e2e-artifacts.nix",
            ["index-a", "tarball-a", "index-b", "tarball-b"],
            {"bosVersionA": "va", "bosVersionB": "vb"},
        )
    ]
    assert run.variant_a is not None and run.variant_a.bos_version == "va"
    assert run.variant_b is not None and run.variant_b.tarball.name == "nix-vb.tar.gz"
    assert run.variant_a.profile_path == "/nix/var/nix/gcroots/profiles/bmc"


def test_build_e2e_artifacts_rejects_version_mismatch(tmp_path: Path) -> None:
    nix = _e2e_nix(tmp_path)
    nix._outs["tarball-a"] = _e2e_tarball_out(tmp_path / "bad", "va", meta_version="other")
    with pytest.raises(Abort, match="carries"):
        catalog.build_e2e_artifacts(nix, _e2e_run(tmp_path))


def test_assemble_rig_records_urls_and_bumped_path(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path)
    nix = _e2e_nix(tmp_path)
    catalog.build_e2e_artifacts(nix, run)
    catalog.assemble_rig(nix, run, workdir=tmp_path / "work", base_url="http://10.1.1.1:8083")
    assert run.bumped_path == "/nix/store/cli-b"
    assert run.rig is not None
    assert run.rig.feed_url == f"http://10.1.1.1:8083/{rig.FEED_NAME}"
    assert run.rig.cache_url == "http://10.1.1.1:8083/cache"
    assert run.rig.cache_public_key == "sysupgrade-e2e-1:PUBLICKEY"
    assert run.rig.preflight_urls == [
        f"http://10.1.1.1:8083/{rig.FEED_NAME}",
        "http://10.1.1.1:8083/tarballs/nix-va.tar.gz",
        f"http://10.1.1.1:8083/index/va/{rig.INDEX_NAME}",
        f"http://10.1.1.1:8083/index/vb/{rig.INDEX_NAME}",
        "http://10.1.1.1:8083/cache/fake.narinfo",
    ]


def _rigged_run(tmp_path: Path) -> catalog.E2eRun:
    run = _e2e_run(tmp_path)
    nix = _e2e_nix(tmp_path)
    catalog.build_e2e_artifacts(nix, run)
    catalog.assemble_rig(nix, run, workdir=tmp_path / "work", base_url="http://10.1.1.1:8083")
    return run


def test_register_rig_writes_factory_then_registers_server(tmp_path: Path) -> None:
    exc = _Exec(_routes({}))
    dev = Device("h", backend=exc)
    catalog.register_rig(dev, _rigged_run(tmp_path))
    write_cmd, register_cmd = (argv[-1] for argv in exc.runs)
    config = json.dumps(
        {
            "factory": {
                "id": "e2e-factory",
                "base_url": "http://10.1.1.1:8083",
                "known_public_key": "sysupgrade-e2e-1:PUBLICKEY",
                "priority": 0,
                "enabled": True,
            },
            "servers": [],
        }
    )
    assert write_cmd == (
        "mkdir -p /etc/nix-upgrade && printf '%s' "
        f"{shlex.quote(config)} > /etc/nix-upgrade/servers.json"
    )
    assert register_cmd == (
        f"{shlex.quote(catalog._REMOTE_CLI)} register-server --id e2e "
        f"--feed-url {shlex.quote(f'http://10.1.1.1:8083/{rig.FEED_NAME}')} "
        "--index-public-key sysupgrade-e2e-1:PUBLICKEY "
        "--cache-url http://10.1.1.1:8083/cache "
        "--cache-public-key sysupgrade-e2e-1:PUBLICKEY"
    )


def test_cleanup_server_registry_removes_the_runtime_file() -> None:
    exc = _Exec(_routes({}))
    catalog.cleanup_server_registry(Device("h", backend=exc))
    assert [argv[-1] for argv in exc.runs] == ["rm -f /etc/nix-upgrade/servers.json"]


def test_preflight_passes_when_all_urls_yield_bytes(tmp_path: Path) -> None:
    exc = _Exec(_routes({"wget": "64"}))
    catalog.preflight_rig(Device("h", backend=exc), _rigged_run(tmp_path))
    probes = [argv[-1] for argv in exc.runs if "wget" in argv[-1]]
    assert len(probes) == 5
    assert all("dd bs=64 count=1" in probe for probe in probes)  # bounded, not full downloads


def test_preflight_aborts_naming_the_unreachable_url(tmp_path: Path) -> None:
    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "wget" in cmd and rig.FEED_NAME not in cmd:
            return _cp(argv, "64")
        return _cp(argv, "0")

    with pytest.raises(Abort, match=rig.FEED_NAME):
        catalog.preflight_rig(Device("h", backend=_Exec(respond)), _rigged_run(tmp_path))


def test_pin_device_address_records_the_numeric_ip(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path)
    catalog.pin_device_address(Device("deck.local"), run, resolve=lambda _h: "10.0.0.9")
    assert run.pinned_host == "10.0.0.9"


def test_pin_device_address_aborts_on_resolution_failure(tmp_path: Path) -> None:
    def resolve(_host: str) -> str:
        raise OSError("no such host")

    with pytest.raises(Abort, match="cannot resolve"):
        catalog.pin_device_address(Device("deck.local"), _e2e_run(tmp_path), resolve=resolve)


_GEN = "/mnt/data/nix/var/nix/profiles/5"


def _cleardown_routes(*, holders: str = "", orchestrator_lingers: bool = False) -> _Respond:
    state = {"unmounted": False, "deleted": False}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "service delete" in cmd:
            state["deleted"] = True
        if "umount" in cmd:
            state["unmounted"] = True
        gone = state["deleted"] and not orchestrator_lingers
        outputs = {
            "service list": "" if gone else '{"bmc-nix-service-orchestrator":{}}',
            "/proc/mounts": "" if state["unmounted"] else "/dev/x /nix ext4 rw 0 0",
            "readlink -f": _GEN,
            "/proc/[0-9]*": holders,
            "[ -d /mnt/data/nix ]": "yes",
        }
        if cmd.startswith("ls"):
            outputs["rc.d"] = "K10foo\nK05baz\nS20bar"
            outputs["init.d"] = "foo\nbar\nbaz"
        for key, value in outputs.items():
            if key in cmd:
                return _cp(argv, value)
        return _cp(argv)

    return respond


def test_cleardown_waits_out_the_orchestrator_before_stopping_services() -> None:
    exc = _Exec(_cleardown_routes())
    catalog.clear_nix_store(Device("h", backend=exc), assume_yes=True)
    cmds = [argv[-1] for argv in exc.runs]
    readlink = next(i for i, c in enumerate(cmds) if "readlink -f" in c)
    delete = next(i for i, c in enumerate(cmds) if "service delete" in c)
    listed = next(i for i, c in enumerate(cmds) if "service list" in c)
    shutdown_baz = cmds.index("/etc/init.d/baz shutdown 2>/dev/null || true")
    shutdown_foo = cmds.index("/etc/init.d/foo shutdown 2>/dev/null || true")
    # The stop pass covers EVERY generation service — the K*-linked ones
    # too, since a shutdown handler may leave its process running.
    stops = [cmds.index(f"/etc/init.d/{n} stop 2>/dev/null || true") for n in ("bar", "baz", "foo")]
    assert readlink < delete < listed < shutdown_baz < shutdown_foo < min(stops)
    sync = cmds.index("sync")
    umount = cmds.index("umount /nix")
    assert max(stops) < sync < umount < cmds.index("rm -rf /mnt/data/nix")


def test_cleardown_aborts_when_the_orchestrator_never_disappears() -> None:
    exc = _Exec(_cleardown_routes(orchestrator_lingers=True))
    clock = iter([0.0, 50.0, 100.0])
    with pytest.raises(Abort, match="orchestrator"):
        catalog.clear_nix_store(
            Device("h", backend=exc),
            assume_yes=True,
            timeout=10,
            sleep=lambda _s: None,
            clock=lambda: next(clock),
        )
    assert not any("shutdown" in argv[-1] or "umount" in argv[-1] for argv in exc.runs)


def test_cleardown_clears_a_stale_orchestrator_without_a_generation() -> None:
    state = {"deleted": False}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "service delete" in cmd:
            state["deleted"] = True
        if "service list" in cmd:
            return _cp(argv, "" if state["deleted"] else '{"bmc-nix-service-orchestrator":{}}')
        if "[ -d /mnt/data/nix ]" in cmd:
            return _cp(argv, "yes")
        return _cp(argv)

    exc = _Exec(respond)
    catalog.clear_nix_store(Device("h", backend=exc), assume_yes=True)
    cmds = [argv[-1] for argv in exc.runs]
    assert any("service delete" in c for c in cmds)
    assert not any("shutdown" in c for c in cmds)  # no generation resolved
    assert any("rm -rf /mnt/data/nix" in c for c in cmds)


def test_cleardown_aborts_while_processes_reference_nix() -> None:
    exc = _Exec(_cleardown_routes(holders="123:bmc-openwrt"))
    clock = iter([0.0, 0.0, 100.0])
    with pytest.raises(Abort, match="bmc-openwrt"):
        catalog.clear_nix_store(
            Device("h", backend=exc),
            assume_yes=True,
            timeout=10,
            sleep=lambda _s: None,
            clock=lambda: next(clock),
        )
    assert not any("umount" in argv[-1] for argv in exc.runs)


def test_cleardown_probes_nix_references_including_bare_targets() -> None:
    exc = _Exec(_cleardown_routes())
    catalog.clear_nix_store(Device("h", backend=exc), assume_yes=True)
    probe = next(argv[-1] for argv in exc.runs if "/proc/[0-9]*" in argv[-1])
    assert probe.count("/nix(/|$)") == 2  # exe/cwd/fd sweep AND the maps grep


def test_cleardown_skips_when_store_already_absent() -> None:
    exc = _Exec(_routes({}))
    catalog.clear_nix_store(Device("h", backend=exc), assume_yes=True)
    assert not any("rm -rf" in argv[-1] for argv in exc.runs)


def test_cleardown_aborts_when_declined(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(catalog.console, "confirm", lambda _q: False)
    with pytest.raises(Abort, match="--yes"):
        catalog.clear_nix_store(Device("h", backend=_Exec(_cleardown_routes())))


def test_cleardown_logs_only_under_dry_run() -> None:
    exc = _Exec(_cleardown_routes())
    token = dry_run.set(True)
    try:
        catalog.clear_nix_store(Device("h", backend=exc), assume_yes=True)
    finally:
        dry_run.reset(token)
    reads = [argv[-1] for argv in exc.runs]
    assert not any("umount" in c or "rm -rf" in c or "stop" in c for c in reads)


def test_flash_e2e_never_skips_on_matching_version(tmp_path: Path) -> None:
    image = _image(tmp_path)
    exc = _Exec(_routes({"bos_version": image.version}))
    catalog.flash_e2e(Device("h", backend=exc), image, assume_yes=True)
    assert any(argv[-1] == f"sysupgrade {shlex.quote(image.remote_path)}" for argv in exc.runs)


def test_flash_e2e_aborts_when_declined(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(catalog.console, "confirm", lambda _q: False)
    with pytest.raises(Abort, match="--yes"):
        catalog.flash_e2e(Device("h", backend=_Exec(_routes({}))), _image(tmp_path))


def test_require_lineage_accepts_image_a_version(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path)
    exc = _Exec(_routes({"bos_version": run.image_a.version}))
    catalog.require_lineage(Device("h", backend=exc), run)


def test_require_lineage_rejects_other_firmware(tmp_path: Path) -> None:
    exc = _Exec(_routes({"bos_version": "something-else"}))
    with pytest.raises(Abort, match="expects image A"):
        catalog.require_lineage(Device("h", backend=exc), _e2e_run(tmp_path))


def test_bump_absent_passes_and_rejects(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    catalog.ensure_bump_absent(Device("h", backend=_Exec(_routes({}))), run)
    exc = _Exec(_routes({"[ -e /nix/store/cli-b ]": "yes"}))
    with pytest.raises(Abort, match="already exists"):
        catalog.ensure_bump_absent(Device("h", backend=exc), run)


def test_record_generation_stores_the_resolved_link(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    exc = _Exec(_routes({"readlink -f": _GEN}))
    catalog.record_generation(Device("h", backend=exc), run)
    assert run.generation_before == _GEN


def test_record_generation_aborts_without_a_generation(tmp_path: Path) -> None:
    with pytest.raises(Abort, match="not initialized"):
        catalog.record_generation(Device("h", backend=_Exec(_routes({}))), _rigged_run(tmp_path))


def test_record_generation_reads_the_variants_profile(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    assert run.variant_b is not None
    run.variant_b = replace(run.variant_b, profile_path="/nix/other/profiles/bmc")
    exc = _Exec(_routes({"readlink -f": _GEN}))
    catalog.record_generation(Device("h", backend=exc), run)
    assert any("/nix/other/profiles/bmc/current" in argv[-1] for argv in exc.runs)


def test_require_uploaded_passes_when_the_device_holds_the_bytes(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path)
    exc = _Exec(_routes({"sha256sum": run.image_a.sha256}))
    catalog.require_uploaded(Device("10.0.0.9", backend=exc), run.image_a)


def test_require_uploaded_aborts_when_the_device_lacks_them(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path)
    with pytest.raises(Abort, match=r"10\.0\.0\.9"):
        catalog.require_uploaded(Device("10.0.0.9", backend=_Exec(_routes({}))), run.image_a)


def test_marker_and_image_sweep_commands(tmp_path: Path) -> None:
    exc = _Exec(_routes({}))
    dev = Device("h", backend=exc)
    run = _e2e_run(tmp_path)
    catalog.drop_e2e_marker(dev)
    catalog.cleanup_e2e_marker(dev)
    catalog.sweep_uploaded_images(dev, run)
    cmds = [argv[-1] for argv in exc.runs]
    assert cmds == [
        "touch /mnt/data/nix/.sysupgrade-e2e-marker",
        "rm -f /mnt/data/nix/.sysupgrade-e2e-marker",
        "rm -f /tmp/a.tar /tmp/b.tar",
    ]


class _PollKnobs(TypedDict):
    timeout: int
    sleep: Callable[[float], None]
    clock: Callable[[], float]


def _instant() -> _PollKnobs:
    """Poll knobs that make verification single-shot in tests."""
    clock = iter([0.0, 100.0, 200.0, 300.0, 400.0])
    return {"timeout": 1, "sleep": lambda _s: None, "clock": lambda: next(clock)}


class _VerifyKnobs(TypedDict, total=False):
    version: Required[str]
    marker: bool
    generation: str
    manifest_hit: bool
    profile_listing: str
    services_ok: bool
    mount_identity: bool


def _verify_routes(**knobs: Unpack[_VerifyKnobs]) -> _Respond:
    version = knobs["version"]
    outputs = {
        "bos_version": version,
        "readlink -f": knobs.get("generation", _GEN),
        "-ef": "yes" if knobs.get("mount_identity", True) else "",
        ".sysupgrade-e2e-marker": "yes" if knobs.get("marker", True) else "",
        "manifest": "yes" if knobs.get("manifest_hit", True) else "",
        "gcroots/profiles/bmc 2>/dev/null": knobs.get("profile_listing", "1-link\ncurrent"),
        " status ": "ok" if knobs.get("services_ok", True) else "",
    }

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "rc.d" in cmd and cmd.startswith("ls"):
            return _cp(argv, "S20bar")
        for key, value in outputs.items():
            if key in cmd:
                return _cp(argv, value)
        return _cp(argv)

    return respond


def test_verify_initialized_passes(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    exc = _Exec(_verify_routes(version="va"))
    catalog.verify_initialized(Device("h", backend=exc), run, **_instant())


def test_verify_initialized_fails_on_version_and_dumps_diagnostics(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    exc = _Exec(_verify_routes(version="wrong"))
    with pytest.raises(Abort, match="bos_version"):
        catalog.verify_initialized(Device("h", backend=exc), run, **_instant())
    cmds = [argv[-1] for argv in exc.runs]
    assert any("grep nix-activator" in c for c in cmds)
    assert any("status >/dev/null 2>&1 && echo running" in c for c in cmds)


def test_verify_initialized_tolerates_a_transport_flap(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    good = _verify_routes(version="va")
    state = {"calls": 0}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        state["calls"] += 1
        if state["calls"] == 1:
            raise subprocess.CalledProcessError(255, argv)
        return good(argv)

    clock = iter([0.0, 100.0, 200.0, 300.0, 400.0])
    knobs: _PollKnobs = {"timeout": 250, "sleep": lambda _s: None, "clock": lambda: next(clock)}
    catalog.verify_initialized(Device("h", backend=_Exec(respond)), run, **knobs)


def test_verify_initialized_survives_an_unreachable_device(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    with pytest.raises(Abort, match="not answering"):
        catalog.verify_initialized(Device("h", backend=_Exec(_unreachable)), run, **_instant())


def test_verify_initialized_fails_on_mount_identity(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    exc = _Exec(_verify_routes(version="va", mount_identity=False))
    with pytest.raises(Abort, match="not backed"):
        catalog.verify_initialized(Device("h", backend=exc), run, **_instant())


def test_verify_initialized_fails_without_a_promoted_current(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    exc = _Exec(_verify_routes(version="va", generation=""))
    with pytest.raises(Abort, match="did not promote"):
        catalog.verify_initialized(Device("h", backend=exc), run, **_instant())


def test_verify_initialized_fails_on_a_stopped_service(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    exc = _Exec(_verify_routes(version="va", services_ok=False))
    with pytest.raises(Abort, match="bar is not running"):
        catalog.verify_initialized(Device("h", backend=exc), run, **_instant())


def test_verify_upgraded_passes(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    run.generation_before = "/mnt/data/nix/var/nix/profiles/4"
    exc = _Exec(_verify_routes(version="vb"))
    catalog.verify_upgraded(Device("h", backend=exc), run, **_instant())


def test_verify_upgraded_survives_an_unreachable_device(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    run.generation_before = "/mnt/data/nix/var/nix/profiles/4"
    with pytest.raises(Abort, match="not answering"):
        catalog.verify_upgraded(Device("h", backend=_Exec(_unreachable)), run, **_instant())


def test_verify_upgraded_fails_when_marker_vanished(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    run.generation_before = "/mnt/data/nix/var/nix/profiles/4"
    exc = _Exec(_verify_routes(version="vb", marker=False))
    with pytest.raises(Abort, match="wiped"):
        catalog.verify_upgraded(Device("h", backend=exc), run, **_instant())


def test_verify_upgraded_fails_when_generation_did_not_advance(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    run.generation_before = _GEN
    exc = _Exec(_verify_routes(version="vb"))
    with pytest.raises(Abort, match="still resolves"):
        catalog.verify_upgraded(Device("h", backend=exc), run, **_instant())


def test_verify_upgraded_fails_when_manifest_misses_the_bump(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    run.generation_before = "/mnt/data/nix/var/nix/profiles/4"
    exc = _Exec(_verify_routes(version="vb", manifest_hit=False))
    with pytest.raises(Abort, match="bumped path"):
        catalog.verify_upgraded(Device("h", backend=exc), run, **_instant())


def test_verify_upgraded_fails_on_pending_next_marker(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    run.generation_before = "/mnt/data/nix/var/nix/profiles/4"
    exc = _Exec(_verify_routes(version="vb", profile_listing="1-link\ncurrent\nnext.vb"))
    with pytest.raises(Abort, match=r"next\.vb"):
        catalog.verify_upgraded(Device("h", backend=exc), run, **_instant())


def test_verify_upgraded_ignores_an_unrelated_next_marker(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    run.generation_before = "/mnt/data/nix/var/nix/profiles/4"
    exc = _Exec(_verify_routes(version="vb", profile_listing="1-link\ncurrent\nnext.other"))
    catalog.verify_upgraded(Device("h", backend=exc), run, **_instant())


def test_rc_name_strips_prefix_and_priority() -> None:
    assert catalog._rc_name("S20bar") == "bar"
    assert catalog._rc_name("K05baz") == "baz"

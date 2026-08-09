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

"""Unit tests for the deploy stage catalog."""

import base64
import hashlib
import io
import json
import shlex
import subprocess
import sys
import tarfile
from collections.abc import Callable, Iterable
from dataclasses import replace
from pathlib import Path
from typing import Literal, Required, TypedDict, Unpack

import pytest

from bmc_tui import catalog, nix_progress, rig
from bmc_tui.device import Device, RemotePath
from bmc_tui.image import Image
from bmc_tui.nix import Attr, Built, Pkg, StorePath
from bmc_tui.procedures.e2e_sysupgrade import E2eSysupgrade
from bmc_tui.procedures.init import Init
from bmc_tui.procedures.sysupgrade import Sysupgrade
from bmc_tui.stage import Abort, dry_run

_TARGET = "stm32mp15/ii3"
_TOP = "sysupgrade-stm32mp15_ii3-emmc"

_Respond = Callable[[list[str]], "subprocess.CompletedProcess[str]"]


def _cp(argv: list[str], stdout: str = "") -> "subprocess.CompletedProcess[str]":
    return subprocess.CompletedProcess(argv, 0, stdout=stdout, stderr="")


class _Exec:
    """Fake Exec: run() delegates to `respond(argv)`; stream() records bytes;
    stream_output() records argv and reports a rebooted (disconnected) flash."""

    def __init__(self, respond: _Respond, *, stream_code: int = 255) -> None:
        self._respond = respond
        self._stream_code = stream_code
        self.runs: list[list[str]] = []
        self.streams: list[tuple[list[str], bytes]] = []
        self.stream_outputs: list[list[str]] = []

    def run(self, argv: list[str]) -> "subprocess.CompletedProcess[str]":
        self.runs.append(argv)
        return self._respond(argv)

    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        self.streams.append((argv, b"".join(chunks)))

    def stream_output(self, argv: list[str], on_line: Callable[[str], None]) -> int:
        self.stream_outputs.append(argv)
        return self._stream_code


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
_HOST_CLI_ATTR = ".#bmc-nix-cli"


class _FakeNix:
    """Attribute-aware fake Nix backend: build_out maps flake attrs to
    prepared output directories. Implements the full Nix protocol so ty
    accepts it wherever a Nix is annotated; the package-oriented methods
    are unused by the init stages."""

    def __init__(self, outs: dict[str, Path]) -> None:
        self._outs = outs
        self.build_file_calls: list[tuple[str, list[Attr], dict[str, str]]] = []

    def discover_widgets(self) -> list[str]:
        return []

    def list_packages(self, prefix: str = "") -> list[str]:
        return []

    def dirty_tree(self) -> bool:
        return False

    def resolve(self, attr: Attr) -> Pkg:
        raise NotImplementedError

    def build(self, pkgs: list[Pkg]) -> list[Built]:
        return []

    def build_out(self, attr: Attr) -> StorePath:
        return StorePath(str(self._outs[attr]))

    def out_path(self, attr: Attr) -> StorePath:
        return StorePath(str(self._outs[attr]))

    def build_file(self, file: str, attrs: list[Attr], args: dict[str, str]) -> list[StorePath]:
        self.build_file_calls.append((file, attrs, args))
        return [StorePath(str(self._outs[attr])) for attr in attrs]

    def generate_cache_key(self, name: str, secret: Path) -> str:
        secret.write_text("sk")
        return f"{name}:PUBLICKEY"

    def copy_signed(self, store_paths: list[StorePath], cache: Path, secret: Path) -> None:
        cache.mkdir(parents=True, exist_ok=True)
        (cache / "fake.narinfo").write_text("StorePath: /nix/store/fake\n")

    def copy(self, store_paths: list[StorePath], dest: str) -> None:
        return None


def _cli_out(tmp_path: Path) -> Path:
    out = tmp_path / "cli-out"
    (out / "bin").mkdir(parents=True, exist_ok=True)
    (out / "bin" / "bmc-nix-cli").write_bytes(b"elf")
    return out


def _stub_host_cli(tmp_path: Path) -> Path:
    out = tmp_path / "host-cli"
    (out / "bin").mkdir(parents=True)
    cli = out / "bin" / "bmc-nix-cli"
    cli.write_text("#!/bin/sh\necho 'sysupgrade-e2e-1:STUBSIG'\n")
    cli.chmod(0o755)
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


def test_memory_shortfall_without_stale_firmware_reports_memory_error() -> None:
    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "MemAvailable" in cmd:
            return _cp(argv, "102924")
        if "ls -1 /tmp" in cmd:
            assert cmd.endswith("|| true"), "an empty glob must not abort the memory check"
        return _cp(argv)

    dev = Device("h", backend=_Exec(respond))
    with pytest.raises(Abort, match="free RAM"):
        catalog.ensure_memory(dev, 200_000_000)


def test_memory_shortfall_reports_stale_firmware_before_asking(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    """A non-interactive run declines the prompt, so the listing has to be printed
    first — otherwise the operator is told there is no RAM but not what took it."""

    monkeypatch.setattr(catalog.console, "confirm", lambda _q: False)
    dev = Device(
        "h", backend=_Exec(_routes({"MemAvailable": "102924", "ls -1 /tmp": "/tmp/old.tar\n"}))
    )

    with pytest.raises(Abort, match="free RAM"):
        catalog.ensure_memory(dev, 200_000_000)

    assert "/tmp/old.tar" in capsys.readouterr().err


def test_memory_recovers_when_the_stale_firmware_is_removed(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Accepting the offer frees the tmpfs the tar held, so the run continues
    instead of making the operator re-invoke it."""

    monkeypatch.setattr(catalog.console, "confirm", lambda _q: True)
    free = iter([100 * 1024 * 1024, 300 * 1024 * 1024])  # before the removal, then after
    backend = _Exec(_routes({"ls -1 /tmp": "/tmp/old.tar\n"}))
    dev = Device("h", backend=backend)
    monkeypatch.setattr(catalog, "_mem_available", lambda _d: next(free))

    catalog.ensure_memory(dev, 200_000_000)

    removed = [" ".join(argv) for argv in backend.runs if "rm -f" in " ".join(argv)]
    assert removed, "the tar must be removed"
    assert "/tmp/old.tar" in removed[0]


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
    assert backend.stream_outputs == []


def test_sysupgrade_runs_with_force(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"cat /etc/bos_version": "older-version"}))
    catalog.sysupgrade(Device("h", backend=backend), image, force=True, assume_yes=True)
    assert any("sysupgrade -F " in argv[-1] for argv in backend.stream_outputs)


def test_sysupgrade_runs_with_assume_yes(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"cat /etc/bos_version": "older-version"}))
    catalog.sysupgrade(Device("h", backend=backend), image, assume_yes=True)
    argv = backend.stream_outputs[0][-1]
    assert argv.startswith("sysupgrade ")  # no BOS_NIX_SKIP prefix by default


def test_sysupgrade_skip_nix_prefixes_env(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"cat /etc/bos_version": "older-version"}))
    catalog.sysupgrade(Device("h", backend=backend), image, assume_yes=True, skip_nix=True)
    assert any("BOS_NIX_SKIP=1 sysupgrade " in argv[-1] for argv in backend.stream_outputs)


def test_cleanup_firmware_removes_uploaded_tar(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({}))
    catalog.cleanup_firmware(Device("h", backend=backend), image)
    assert any(f"rm -f {shlex.quote(image.remote_path)}" in argv[-1] for argv in backend.runs)


def test_sysupgrade_cleans_up_tmp_after_flash_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(nix_progress.tempfile, "gettempdir", lambda: str(tmp_path))
    image = _image(tmp_path)
    backend = _Exec(
        _routes(
            {
                "ubus call system board": json.dumps(
                    {"board_name": "b", "release": {"target": _TARGET}}
                ),
                "MemAvailable": "9999999",
                "sha256sum": image.sha256,
                "cat /etc/bos_version": "older-version",
            }
        ),
        stream_code=1,  # flash fails without rebooting → the tar stays in /tmp
    )
    monkeypatch.setattr(
        "bmc_tui.procedures.sysupgrade.Device", lambda host: Device(host, backend=backend)
    )

    with pytest.raises(Abort):
        Sysupgrade(device="h", image=image.path, yes=True).run()

    assert any(f"rm -f {shlex.quote(image.remote_path)}" in argv[-1] for argv in backend.runs)


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


def test_plant_stale_next_marker_creates_a_symlink() -> None:
    """The device sweep (sweep_next_markers) mirrors the shell activator's
    `[ -L ]` guard and deliberately skips non-symlinks, so a real stale marker
    — and this plant — must be a symlink. A touched regular file would survive
    activation and false-fail require_stale_next_gone on correct hardware."""
    exec_ = _Exec(_routes({}))
    catalog.plant_stale_next_marker(Device("h", backend=exec_))
    planted = [argv[-1] for argv in exec_.runs if "next.9999" in argv[-1]]
    assert planted, "the stale marker was never planted"
    assert all("ln -s" in cmd for cmd in planted), planted
    assert not any(cmd.startswith("touch ") for cmd in planted)


# ── nix package deploy ────────────────────────────────────────────────────────


class _Nix:
    """Fake Nix: resolve names from the attr leaf, build to a fake store path."""

    def __init__(
        self,
        widgets: tuple[str, ...] = (),
        out_dir: str = "",
        packages: tuple[str, ...] = (),
        dirty: bool = False,
    ) -> None:
        self.widgets = list(widgets)
        self.packages = list(packages) or ["core", *self.widgets]
        self.dirty = dirty
        self.built: list[Pkg] = []
        self.copied: list[tuple[list[StorePath], str]] = []
        self.out_dir = out_dir
        self.listed_prefixes: list[str] = []

    def discover_widgets(self) -> list[str]:
        return list(self.widgets)

    def list_packages(self, prefix: str = "") -> list[str]:
        self.listed_prefixes.append(prefix)
        return list(self.packages)

    def dirty_tree(self) -> bool:
        return self.dirty

    def build_out(self, attr: Attr) -> StorePath:
        return StorePath(self.out_dir)

    def out_path(self, attr: Attr) -> StorePath:
        return StorePath(self.out_dir)

    def build_file(self, file: str, attrs: list[Attr], args: dict[str, str]) -> list[StorePath]:
        raise NotImplementedError

    def generate_cache_key(self, name: str, secret: Path) -> str:
        raise NotImplementedError

    def copy_signed(self, store_paths: list[StorePath], cache: Path, secret: Path) -> None:
        raise NotImplementedError

    def resolve(self, attr: Attr) -> Pkg:
        name = attr.rsplit(".", 1)[-1]
        return Pkg(name=name, version="1.0.0", installable=Attr(f"{attr}.pkg^out"))

    def build(self, pkgs: list[Pkg]) -> list[Built]:
        self.built.extend(pkgs)
        return [
            Built(
                pkg.name,
                pkg.version,
                pkg.installable,
                store_path=StorePath(f"/nix/store/{pkg.name}"),
            )
            for pkg in pkgs
        ]

    def copy(self, store_paths: list[StorePath], dest: str) -> None:
        self.copied.append((store_paths, dest))


def test_resolve_discovers_core_plus_widgets() -> None:
    plan = catalog.Deployment(attrs=[])
    catalog.resolve_packages(_Nix(widgets=("clock", "weather")), plan)
    assert [p.name for p in plan.resolved] == ["core", "bmc-nix-cli", "clock", "weather"]
    assert plan.attrs[0] == ".#deck-packages.core"


def test_resolve_uses_explicit_packages() -> None:
    plan = catalog.Deployment(attrs=[Attr(".#deck-packages.core")])
    catalog.resolve_packages(_Nix(widgets=("clock",)), plan)  # widgets ignored when explicit
    assert [p.name for p in plan.resolved] == ["core"]


# Verbatim from `nix eval` on this repo — the classifier keys off these words, so a
# paraphrase here would test nothing.
_ABSENT_STDERR = (
    "error: flake 'git+file:///repo' does not provide attribute "
    "'packages.x86_64-linux.deck-packages.image'"
)
_LFS_STDERR = (
    'error: bad json from /info/lfs/objects/batch: {"error":{"code":404,'
    '"message":"Object does not exist on the server"},'
    '"oid":"75a6f686f8bbcb4f132d6ab7240822269d52146114aadf5d668270b0493badb9","size":1596}'
)


def _failing_nix(
    stderr: str, *, widgets: tuple[str, ...] = (), packages: tuple[str, ...] = ()
) -> _Nix:
    """A fake whose `resolve` dies the way `_eval_raw` does, stderr included."""

    class _BadNix(_Nix):
        def resolve(self, attr: Attr) -> Pkg:
            raise subprocess.CalledProcessError(1, ["nix", "eval", attr], stderr=stderr)

    return _BadNix(widgets=widgets, packages=packages)


def test_resolve_aborts_with_suggestion_on_unknown_package() -> None:
    plan = catalog.Deployment(attrs=[Attr(".#deck-packages.image")])
    nix = _failing_nix(_ABSENT_STDERR, widgets=("widget-image", "widget-clock"))
    with pytest.raises(Abort, match="widget-image"):
        catalog.resolve_packages(nix, plan)


def test_resolve_suggests_non_widget_packages() -> None:
    plan = catalog.Deployment(attrs=[Attr(".#deck-packages.frontend")])
    nix = _failing_nix(_ABSENT_STDERR, packages=("core", "bmc-frontend", "widget-image"))
    with pytest.raises(Abort, match="bmc-frontend"):
        catalog.resolve_packages(nix, plan)


def test_resolve_suggests_from_the_profile_being_deployed() -> None:
    """A release-set suggestion under a debug prefix can name the rejected attr back."""
    plan = catalog.Deployment(attrs=[Attr("image")], prefix=catalog.package_prefix("debug"))
    nix = _failing_nix(_ABSENT_STDERR, widgets=("widget-image",))
    with pytest.raises(Abort):
        catalog.resolve_packages(nix, plan)
    assert nix.listed_prefixes == [".#deck-packages-debug"]


def test_resolve_panels_an_unpushed_lfs_object_with_its_oid(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """The eval failure is about a missing object, not a missing package."""
    plan = catalog.Deployment(attrs=[Attr(".#deck-packages.core")])
    with pytest.raises(Abort, match="git-LFS") as caught:
        catalog.resolve_packages(_failing_nix(_LFS_STDERR), plan)
    panel = capsys.readouterr().out
    assert "75a6f686" in panel
    assert "git lfs push" in panel
    assert "does not exist" not in str(caught.value), "must not read as a typo"


def test_resolve_panels_an_unrecognised_nix_error_verbatim(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """Inventing a diagnosis for an unclassified failure is what hid the last one."""
    plan = catalog.Deployment(attrs=[Attr(".#deck-packages.core")])
    with pytest.raises(Abort, match="failed to evaluate"):
        catalog.resolve_packages(_failing_nix("error: cannot connect to the daemon"), plan)
    assert "daemon" in capsys.readouterr().out


def test_resolve_keeps_nix_warnings_out_of_the_panel(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """A dirty tree prefixes every nix stderr; padding the report with it buries it."""
    stderr = "warning: Git tree '/repo' has uncommitted changes\nerror: no space left"
    plan = catalog.Deployment(attrs=[Attr(".#deck-packages.core")])
    with pytest.raises(Abort):
        catalog.resolve_packages(_failing_nix(stderr), plan)
    panel = capsys.readouterr().out
    assert "space" in panel
    assert "uncommitted" not in panel


def test_resolve_does_not_let_rich_eat_a_bracketed_nix_span(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """nix's errors carry `[json.exception…]` spans that rich would read as markup."""
    plan = catalog.Deployment(attrs=[Attr(".#deck-packages.core")])
    with pytest.raises(Abort):
        catalog.resolve_packages(_failing_nix("error: bad [json.exception.out_of_range.403]"), plan)
    assert "json.exception" in capsys.readouterr().out


def test_resolve_states_a_dirty_worktree_once(capsys: pytest.CaptureFixture[str]) -> None:
    """The build calls suppress nix's own notice, so the TUI owes the user this."""
    catalog.resolve_packages(_Nix(dirty=True), catalog.Deployment(attrs=[Attr("core")]))
    assert capsys.readouterr().err.count("uncommitted changes") == 1


def test_resolve_says_nothing_when_the_worktree_is_clean(
    capsys: pytest.CaptureFixture[str],
) -> None:
    catalog.resolve_packages(_Nix(), catalog.Deployment(attrs=[Attr("core")]))
    assert "uncommitted" not in capsys.readouterr().err


def test_resolve_qualifies_bare_package_names() -> None:
    plan = catalog.Deployment(attrs=[Attr("core"), Attr("widget-image")])
    catalog.resolve_packages(_Nix(), plan)
    assert plan.attrs == [".#deck-packages.core", ".#deck-packages.widget-image"]


def test_resolve_leaves_qualified_attrs_alone() -> None:
    plan = catalog.Deployment(attrs=[Attr(".#armv7-nixpkgs.strace")])
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
    plan = catalog.Deployment(attrs=[], resolved=[Pkg("core", "1.0", Attr(".#x.pkg^out"))])
    catalog.build_packages(_Nix(), plan)
    assert [b.store_path for b in plan.built] == ["/nix/store/core"]


def test_copy_closures_sends_built_paths() -> None:
    nix = _Nix()
    dev = Device("h", backend=_Exec(_routes({})))
    plan = catalog.Deployment(
        attrs=[], built=[Built("core", "1.0", Attr(".#x.pkg^out"), StorePath("/nix/store/core"))]
    )
    catalog.copy_closures(nix, dev, plan)
    assert nix.copied == [(["/nix/store/core"], dev.copy_dest)]


def test_register_packages_builds_cli_command() -> None:
    backend = _Exec(_routes({}))
    dev = Device("h", backend=backend)
    plan = catalog.Deployment(
        attrs=[], built=[Built("core", "1.0", Attr(".#x.pkg^out"), StorePath("/nix/store/core"))]
    )
    catalog.register_packages(dev, plan)
    cmd = backend.runs[-1][-1]
    assert "bmc-nix-cli add-packages" in cmd
    assert "--name core --version 1.0 --store-path /nix/store/core" in cmd


def test_clear_upgrade_servers_invokes_the_profile_cli() -> None:
    backend = _Exec(_routes({}))
    dev = Device("h", backend=backend)
    catalog.clear_upgrade_servers(dev)
    cmd = backend.runs[-1][-1]
    assert cmd == f"{catalog._NIX_CLI} clear-servers"


def test_clear_upgrade_servers_dry_run_skips_the_device() -> None:
    backend = _Exec(_routes({}))
    dev = Device("h", backend=backend)
    token = dry_run.set(True)
    try:
        catalog.clear_upgrade_servers(dev)
    finally:
        dry_run.reset(token)
    assert backend.runs == [], "dry-run must log the mutation without executing it"


def test_clear_upgrade_servers_tolerates_a_cli_without_the_subcommand() -> None:
    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        raise subprocess.CalledProcessError(
            2, argv, stderr="error: unrecognized subcommand 'clear-servers'"
        )

    dev = Device("h", backend=_Exec(respond))
    catalog.clear_upgrade_servers(dev)


def test_clear_upgrade_servers_raises_on_other_failures() -> None:
    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        raise subprocess.CalledProcessError(1, argv, stderr="io error")

    dev = Device("h", backend=_Exec(respond))
    with pytest.raises(subprocess.CalledProcessError):
        catalog.clear_upgrade_servers(dev)


_DEFAULT_REGISTRY = json.dumps(
    {
        "factory": {
            "id": "factory",
            "base_url": "https://factory.example",
            "known_public_key": "factory:key",
            "priority": 10,
            "enabled": True,
        },
        "servers": [
            {
                "id": "forge",
                "feed_url": "https://downloads.braiinsforge.com/feed.json",
                "known_public_key": "forge:key",
                "priority": 50,
                "enabled": True,
                "required": True,
            }
        ],
    }
)


def test_register_default_servers_replays_the_default_entry() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": _DEFAULT_REGISTRY}))
    dev = Device("h", backend=backend)
    catalog.register_default_servers(dev)
    cmd = backend.runs[-1][-1]
    assert f"{catalog._NIX_CLI} register-server" in cmd
    assert "--id forge" in cmd
    assert "--feed-url https://downloads.braiinsforge.com/feed.json" in cmd
    assert "--index-public-key forge:key" in cmd
    assert "--priority 50" in cmd
    assert "--cache-url" not in cmd
    assert "--optional" not in cmd


def test_register_default_servers_url_override_replaces_the_source() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": _DEFAULT_REGISTRY}))
    dev = Device("h", backend=backend)
    catalog.register_default_servers(dev, url="https://staging.example/feed.json")
    cmd = backend.runs[-1][-1]
    assert "--feed-url https://staging.example/feed.json" in cmd
    assert "--index-public-key forge:key" in cmd


def test_register_default_servers_without_default_needs_explicit_fields() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": ""}))
    dev = Device("h", backend=backend)
    with pytest.raises(Abort):
        catalog.register_default_servers(dev)


def test_register_default_servers_explicit_entry_without_default() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": ""}))
    dev = Device("h", backend=backend)
    catalog.register_default_servers(
        dev,
        url="https://dev.example/index.json",
        entry_id="dev",
        key="dev:key",
    )
    cmd = backend.runs[-1][-1]
    assert "--id dev" in cmd
    assert "--index-url https://dev.example/index.json" in cmd
    assert "--index-public-key dev:key" in cmd


def test_register_default_servers_factory_only_uses_explicit_entry() -> None:
    registry = json.loads(_DEFAULT_REGISTRY)
    del registry["servers"]
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": json.dumps(registry)}))
    catalog.register_default_servers(
        Device("h", backend=backend),
        url="https://dev.example/index.json",
        entry_id="dev",
        key="dev:key",
    )
    commands = [run[-1] for run in backend.runs if "register-server" in run[-1]]
    assert len(commands) == 1
    assert "--id dev" in commands[0]
    assert "--index-url https://dev.example/index.json" in commands[0]
    assert "--index-public-key dev:key" in commands[0]


_TWO_ENTRY_REGISTRY = json.dumps(
    {
        "factory": {
            "id": "factory",
            "base_url": "https://factory.example",
            "known_public_key": "factory:key",
            "priority": 10,
            "enabled": True,
        },
        "servers": [
            {
                "id": "forge",
                "feed_url": "https://downloads.braiinsforge.com/feed.json",
                "known_public_key": "forge:key",
                "priority": 50,
                "enabled": True,
                "required": True,
            },
            {
                "id": "mirror",
                "index_url": "https://mirror.example/index.json",
                "known_public_key": "mirror:key",
                "priority": 80,
                "enabled": True,
                "required": False,
            },
        ],
    }
)


def test_register_default_servers_replays_every_entry_faithfully() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": _TWO_ENTRY_REGISTRY}))
    dev = Device("h", backend=backend)
    catalog.register_default_servers(dev)
    cmds = [run[-1] for run in backend.runs if "register-server" in run[-1]]
    assert len(cmds) == 2
    assert "--feed-url https://downloads.braiinsforge.com/feed.json" in cmds[0]
    assert "--priority 50" in cmds[0]
    assert "--optional" not in cmds[0]
    assert "--index-url https://mirror.example/index.json" in cmds[1]
    assert "--priority 80" in cmds[1]
    assert "--optional" in cmds[1], "required: false must replay as --optional"


def test_register_default_servers_skips_a_disabled_entry() -> None:
    registry = json.loads(_TWO_ENTRY_REGISTRY)
    registry["servers"][1]["enabled"] = False
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": json.dumps(registry)}))
    dev = Device("h", backend=backend)
    catalog.register_default_servers(dev)
    cmds = [run[-1] for run in backend.runs if "register-server" in run[-1]]
    assert len(cmds) == 1, "a disabled default entry must not be replayed"
    assert "--id forge" in cmds[0]


def test_register_default_servers_rejects_an_id_naming_a_disabled_entry() -> None:
    registry = json.loads(_TWO_ENTRY_REGISTRY)
    registry["servers"][1]["enabled"] = False
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": json.dumps(registry)}))
    with pytest.raises(Abort, match="'mirror' is disabled"):
        catalog.register_default_servers(
            Device("h", backend=backend), url="https://x.example", entry_id="mirror"
        )
    assert not any("register-server" in run[-1] for run in backend.runs), (
        "--id naming a disabled entry must not register another server under that id"
    )


def test_register_default_servers_aborts_when_every_entry_is_disabled() -> None:
    registry = json.loads(_TWO_ENTRY_REGISTRY)
    for entry in registry["servers"]:
        entry["enabled"] = False
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": json.dumps(registry)}))
    with pytest.raises(Abort, match="no enabled server entries"):
        catalog.register_default_servers(Device("h", backend=backend))
    assert not any("register-server" in run[-1] for run in backend.runs), (
        "a fully disabled registry must not be replayed as enabled"
    )


def test_register_default_servers_id_selects_among_multiple_entries() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": _TWO_ENTRY_REGISTRY}))
    dev = Device("h", backend=backend)
    catalog.register_default_servers(
        dev, url="https://staging.example/index.json", entry_id="mirror"
    )
    cmds = [run[-1] for run in backend.runs if "register-server" in run[-1]]
    assert len(cmds) == 1
    assert "--id mirror" in cmds[0]
    assert "--index-url https://staging.example/index.json" in cmds[0]


def test_register_default_servers_rejects_an_unknown_id() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": _TWO_ENTRY_REGISTRY}))
    dev = Device("h", backend=backend)
    with pytest.raises(Abort):
        catalog.register_default_servers(dev, url="https://x.example", entry_id="nope")


def test_register_default_servers_url_alone_is_ambiguous_with_two_entries() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": _TWO_ENTRY_REGISTRY}))
    dev = Device("h", backend=backend)
    with pytest.raises(Abort):
        catalog.register_default_servers(dev, url="https://x.example")


def test_register_server_cmd_quotes_a_metacharacter_priority() -> None:
    priority = "50; touch /tmp/owned"
    cmd = catalog._register_server_cmd(
        {
            "id": "forge",
            "feed_url": "https://downloads.braiinsforge.com/feed.json",
            "known_public_key": "forge:key",
            "priority": priority,
        }
    )
    assert shlex.split(cmd)[-1] == priority


def test_register_default_servers_rejects_a_metacharacter_priority() -> None:
    registry = json.loads(_DEFAULT_REGISTRY)
    registry["servers"][0]["priority"] = "50; touch /tmp/owned"
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": json.dumps(registry)}))
    with pytest.raises(Abort, match=r"servers\[0\]\.priority"):
        catalog.register_default_servers(Device("h", backend=backend))
    assert not any("register-server" in run[-1] for run in backend.runs)


def test_register_default_servers_rejects_malformed_json_without_mutation() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": "{"}))
    with pytest.raises(Abort, match="not valid JSON"):
        catalog.register_default_servers(Device("h", backend=backend))
    assert not any("register-server" in run[-1] for run in backend.runs)


@pytest.mark.parametrize("raw", ["null", "[]", '"registry"'])
def test_register_default_servers_rejects_a_non_object_root(raw: str) -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": raw}))
    with pytest.raises(Abort, match="must contain a JSON object"):
        catalog.register_default_servers(Device("h", backend=backend))
    assert not any("register-server" in run[-1] for run in backend.runs)


@pytest.mark.parametrize("servers", [None, {}, "entries"])
def test_register_default_servers_requires_a_servers_list(servers: object) -> None:
    raw = json.dumps({"servers": servers})
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": raw}))
    with pytest.raises(Abort, match="must contain a servers list"):
        catalog.register_default_servers(Device("h", backend=backend))
    assert not any("register-server" in run[-1] for run in backend.runs)


def test_register_default_servers_validates_every_entry_before_mutating() -> None:
    registry = json.loads(_TWO_ENTRY_REGISTRY)
    del registry["servers"][1]["known_public_key"]
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": json.dumps(registry)}))
    with pytest.raises(Abort, match=r"servers\[1\]\.known_public_key"):
        catalog.register_default_servers(Device("h", backend=backend))
    assert not any("register-server" in run[-1] for run in backend.runs)


@pytest.mark.parametrize(
    ("sources", "message"),
    [
        ({}, "exactly one"),
        ({"feed_url": None}, "exactly one"),
        (
            {
                "feed_url": "https://downloads.braiinsforge.com/feed.json",
                "index_url": "https://downloads.braiinsforge.com/index.json",
            },
            "exactly one",
        ),
    ],
)
def test_register_default_servers_requires_exactly_one_source(
    sources: dict[str, object], message: str
) -> None:
    registry = json.loads(_DEFAULT_REGISTRY)
    entry = registry["servers"][0]
    entry.pop("feed_url")
    entry.update(sources)
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": json.dumps(registry)}))
    with pytest.raises(Abort, match=message):
        catalog.register_default_servers(Device("h", backend=backend))
    assert not any("register-server" in run[-1] for run in backend.runs)


def test_register_default_servers_treats_a_null_source_as_absent() -> None:
    registry = json.loads(_DEFAULT_REGISTRY)
    entry = registry["servers"][0]
    entry["feed_url"] = None
    entry["index_url"] = "https://downloads.braiinsforge.com/index.json"
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": json.dumps(registry)}))
    catalog.register_default_servers(Device("h", backend=backend))
    commands = [run[-1] for run in backend.runs if "register-server" in run[-1]]
    assert len(commands) == 1
    assert "--index-url https://downloads.braiinsforge.com/index.json" in commands[0]
    assert "--feed-url" not in commands[0]


def test_register_default_servers_dry_run_reads_but_skips_mutations() -> None:
    backend = _Exec(_routes({f"cat {catalog._SERVERS_JSON_DEFAULT}": _DEFAULT_REGISTRY}))
    token = dry_run.set(True)
    try:
        catalog.register_default_servers(Device("h", backend=backend))
    finally:
        dry_run.reset(token)
    assert len(backend.runs) == 1
    assert f"cat {catalog._SERVERS_JSON_DEFAULT}" in backend.runs[0][-1]


def _flip_clock_plan() -> catalog.Deployment:
    built = Built(
        "widget-flip-clock", "1.0", Attr(".#x^out"), store_path=StorePath("/nix/store/wfc")
    )
    return catalog.Deployment(attrs=[], built=[built])


def test_remove_legacy_flip_clock_issues_remove_command() -> None:
    backend = _Exec(_routes({}))
    catalog.remove_legacy_flip_clock(Device("h", backend=backend), _flip_clock_plan())
    assert "remove-packages --name flip-clock" in backend.runs[-1][-1]


def test_remove_legacy_flip_clock_skips_without_successor() -> None:
    # No widget-flip-clock in the deploy → no conflict → nothing to remove.
    backend = _Exec(_routes({}))
    plan = catalog.Deployment(
        attrs=[],
        built=[Built("core", "1.0", Attr(".#x^out"), store_path=StorePath("/nix/store/core"))],
    )
    catalog.remove_legacy_flip_clock(Device("h", backend=backend), plan)
    assert backend.runs == []


def test_remove_legacy_flip_clock_tolerates_failure() -> None:
    # A device that never had the legacy package makes remove-packages exit
    # non-zero; the deploy must swallow it rather than abort.
    catalog.remove_legacy_flip_clock(Device("h", backend=_Exec(_unreachable)), _flip_clock_plan())


def test_stop_compositor_stops_the_profile_service() -> None:
    backend = _Exec(_routes({}))
    catalog.stop_compositor(Device("h", backend=backend))
    assert "service bmc-compositor stop" in backend.runs[0][-1]


def test_stop_compositor_waits_for_the_wasm_hosts_to_exit() -> None:
    """The service stop returns while the wasm hosts are still exiting;
    reporting stopped before they are gone lets the memory gate sample RAM
    they still hold and abort with a bogus insufficient-memory failure."""
    lingering = {"polls_left": 2}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        if "pidof bmc-wasm-host" in argv[-1]:
            lingering["polls_left"] -= 1
            return _cp(argv, "321" if lingering["polls_left"] > 0 else "")
        return _cp(argv)

    backend = _Exec(respond)
    catalog.stop_compositor(
        Device("h", backend=backend), sleep=lambda _s: None, clock=_ticking_clock()
    )
    assert lingering["polls_left"] == 0  # returned only once the hosts were gone


def test_stop_compositor_aborts_when_a_wasm_host_lingers() -> None:
    backend = _Exec(_routes({"pidof bmc-wasm-host": "321"}))
    with pytest.raises(Abort, match="bmc-wasm-host"):
        catalog.stop_compositor(
            Device("h", backend=backend), timeout=4, sleep=lambda _s: None, clock=_ticking_clock()
        )


def _restarted(backend: _Exec) -> bool:
    return any("/etc/init.d/bmc-compositor restart" in argv[-1] for argv in backend.runs)


def test_restart_compositor_restarts_by_default() -> None:
    backend = _Exec(_routes({}))
    catalog.restart_compositor(Device("h", backend=backend))
    assert _restarted(backend)


def test_restart_compositor_skips_on_request() -> None:
    backend = _Exec(_routes({}))
    catalog.restart_compositor(Device("h", backend=backend), skip=True)
    assert backend.runs == []


def test_restart_compositor_defers_to_the_orchestrator() -> None:
    """A pid that moved across the activation means the reload already ran."""
    backend = _Exec(_routes({"pidof": "999"}))
    catalog.restart_compositor(Device("h", backend=backend), old_pid=catalog.Pid("111"))
    assert not _restarted(backend)


def test_restart_compositor_restarts_when_the_pid_held() -> None:
    """An unchanged pid means only widgets moved, so nothing reloaded them."""
    backend = _Exec(_routes({"pidof": "111"}))
    catalog.restart_compositor(Device("h", backend=backend), old_pid=catalog.Pid("111"))
    assert _restarted(backend)


def test_restart_compositor_skips_under_dry_run() -> None:
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


def test_ensure_nix_cli_skips_when_exclusive_registration_is_supported() -> None:
    nix = _Nix()
    dev = Device("h", backend=_Exec(_routes({"register-server --help": "ok"})))
    catalog.ensure_nix_cli(nix, dev)
    assert not nix.built  # no bootstrap needed


def test_ensure_nix_cli_bootstraps_when_absent() -> None:
    state = {"present": False}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "add-packages" in cmd:
            state["present"] = True
        if "register-server --help" in cmd:
            return _cp(argv, "ok" if state["present"] else "")
        return _cp(argv)

    nix = _Nix()
    catalog.ensure_nix_cli(nix, Device("h", backend=_Exec(respond)))
    assert [p.name for p in nix.built] == ["bmc-nix-cli"]
    assert nix.copied  # the closure was shipped


def test_ensure_nix_cli_replaces_one_without_exclusive_registration() -> None:
    state = {"compatible": False}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "add-packages" in cmd:
            state["compatible"] = True
        if "register-server --help" in cmd:
            return _cp(argv, "ok" if state["compatible"] else "")
        return _cp(argv)

    nix = _Nix()
    catalog.ensure_nix_cli(nix, Device("h", backend=_Exec(respond)))
    assert [p.name for p in nix.built] == ["bmc-nix-cli"]


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
    plan.remote_tarball = RemotePath("/mnt/data/nix-x.tar.gz.deck-init.1")
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
    # the B2 debugfs is swept alongside the CLI and tarball
    assert any("rm -f" in c and catalog._REMOTE_DEBUGFS in c for c in joined)


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


def test_register_upgrade_server_registers_index_document_url() -> None:
    # Regression: the register-server rename from --base-url to --index-url
    # also changed URL semantics — --index-url is fetched verbatim rather than
    # having the filename appended, so the harness must register the index
    # document itself, not the bare serving base.
    backend = _Exec(_routes({}))
    cycle = _cycle()
    cycle.host = "10.0.0.20"
    cycle.cache_public_key = "dev-upgrade:KEY"
    catalog.register_upgrade_server(Device("h", backend=backend), cycle)
    cmd = backend.runs[-1][-1]
    assert "--base-url" not in cmd
    assert (
        f"register-server --exclusive --id {catalog._UPGRADE_SERVER_ID} "
        "--index-url http://10.0.0.20:8081/nix-package-index.v1.json "
        "--index-public-key dev-upgrade:KEY "
        "--cache-url http://10.0.0.20:8080 "
        "--cache-public-key dev-upgrade:KEY"
    ) in cmd


def _registry(*servers: dict[str, object]) -> str:
    return json.dumps({"factory": {"id": "factory"}, "servers": list(servers)})


def _unclaimed_probe(raw: str, *, nix_conf: str = "") -> _Respond:
    return _routes(
        {
            f"if [ -e {catalog.SERVERS_JSON} ]; then cat {catalog.SERVERS_JSON}; fi": raw,
            f"if [ -e {catalog._NIX_CONF} ]; then cat {catalog._NIX_CONF}; fi": nix_conf,
        }
    )


def test_unclaimed_registry_rejects_a_predecessors_leftover_entry() -> None:
    # Capturing a predecessor's registry as "the pre-run state" hands it back
    # as the device's baseline, and every later run does the same — the Deck
    # would resolve no upgrade ever again.
    raw = _registry(
        {"id": "forge", "enabled": False},
        {"id": catalog._UPGRADE_SERVER_ID, "enabled": True},
    )
    backend = _Exec(_unclaimed_probe(raw))
    with pytest.raises(Abort, match="did not restore the dev-upgrade package config"):
        catalog.require_unclaimed_package_registry(Device("h", backend=backend))


def test_unclaimed_registry_rejects_a_leftover_entry_left_disabled() -> None:
    # Disabled is just as poisonous: the restore would still hand back a
    # registry whose production servers a predecessor turned off.
    raw = _registry({"id": catalog._UPGRADE_SERVER_ID, "enabled": False})
    backend = _Exec(_unclaimed_probe(raw))
    with pytest.raises(Abort, match="did not restore the dev-upgrade package config"):
        catalog.require_unclaimed_package_registry(Device("h", backend=backend))


def test_unclaimed_registry_rejects_a_leftover_trust_key_without_an_entry() -> None:
    nix_conf = "\n".join(
        (
            "extra-substituters = https://cache.example.com http://192.0.2.1:8080",
            "extra-trusted-public-keys = cache.example.com:KEY dev-upgrade:STALE",
        )
    )
    backend = _Exec(
        _unclaimed_probe(_registry({"id": "forge", "enabled": True}), nix_conf=nix_conf)
    )
    with pytest.raises(Abort, match="extra-trusted-public-keys"):
        catalog.require_unclaimed_package_registry(Device("h", backend=backend))


def test_unclaimed_registry_accepts_unrelated_trusted_keys() -> None:
    nix_conf = "extra-trusted-public-keys = cache.example.com:KEY"
    backend = _Exec(
        _unclaimed_probe(_registry({"id": "forge", "enabled": True}), nix_conf=nix_conf)
    )
    catalog.require_unclaimed_package_registry(Device("h", backend=backend))


def test_unclaimed_registry_accepts_a_production_only_registry() -> None:
    raw = _registry({"id": "forge", "enabled": True})
    backend = _Exec(_unclaimed_probe(raw))
    catalog.require_unclaimed_package_registry(Device("h", backend=backend))


def test_unclaimed_registry_accepts_an_absent_registry() -> None:
    # register-server seeds the runtime file from the shipped default, so a
    # device that never registered anything is a legitimate starting point.
    backend = _Exec(_unclaimed_probe(""))
    catalog.require_unclaimed_package_registry(Device("h", backend=backend))


def test_unclaimed_registry_aborts_on_malformed_json() -> None:
    backend = _Exec(_unclaimed_probe("not json"))
    with pytest.raises(Abort, match="not valid JSON before registration"):
        catalog.require_unclaimed_package_registry(Device("h", backend=backend))


def test_exclusive_package_server_accepts_a_disabled_public_entry() -> None:
    raw = _registry(
        {"id": "forge", "enabled": False},
        {"id": catalog._UPGRADE_SERVER_ID, "enabled": True},
    )
    backend = _Exec(_routes({f"cat {catalog.SERVERS_JSON}": raw}))
    catalog.require_exclusive_package_server(Device("h", backend=backend))


def test_exclusive_package_server_rejects_a_live_public_entry() -> None:
    # The point of --exclusive: resolution ranks a candidate's version above
    # its server's priority, so an enabled forge publishing something newer
    # decides the upgrade instead of the harness rig.
    raw = _registry(
        {"id": "forge", "enabled": True},
        {"id": catalog._UPGRADE_SERVER_ID, "enabled": True},
    )
    backend = _Exec(_routes({f"cat {catalog.SERVERS_JSON}": raw}))
    with pytest.raises(Abort, match="only enabled"):
        catalog.require_exclusive_package_server(Device("h", backend=backend))


def test_exclusive_package_server_treats_a_missing_enabled_flag_as_live() -> None:
    raw = _registry({"id": "forge"}, {"id": catalog._UPGRADE_SERVER_ID, "enabled": True})
    backend = _Exec(_routes({f"cat {catalog.SERVERS_JSON}": raw}))
    with pytest.raises(Abort, match="only enabled"):
        catalog.require_exclusive_package_server(Device("h", backend=backend))


def test_exclusive_package_server_aborts_on_malformed_json() -> None:
    backend = _Exec(_routes({f"cat {catalog.SERVERS_JSON}": "not json"}))
    with pytest.raises(Abort, match="not valid JSON after registration"):
        catalog.require_exclusive_package_server(Device("h", backend=backend))


def _manifest(**paths: str) -> str:
    return json.dumps({"packages": {name: {"store_path": p} for name, p in paths.items()}})


def test_verify_profile_advanced_ok() -> None:
    cycle = _cycle(generation_before=5, installed_before={"core"})
    built = Built("core", "1.0", Attr(".#x"), store_path=StorePath("/nix/store/abc-core"))
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
    built = Built("core", "1.0", Attr(".#x"), store_path=StorePath("/nix/store/abc-core"))
    manifest = _manifest(core="/nix/store/other")
    backend = _Exec(_routes({"readlink": "6-link", "cat": manifest}))
    plan = catalog.Deployment(attrs=[], built=[built])
    with pytest.raises(Abort, match="not replaced by their served store paths"):
        catalog.verify_profile_advanced(Device("h", backend=backend), plan, cycle)


def test_verify_profile_advanced_aborts_on_path_under_wrong_package() -> None:
    cycle = _cycle(generation_before=5, installed_before={"core"})
    built = Built("core", "1.0", Attr(".#x"), store_path=StorePath("/nix/store/abc-core"))
    # The served path is present, but attached to a different package — a
    # substring check would have wrongly passed.
    manifest = _manifest(other="/nix/store/abc-core", core="/nix/store/stale-core")
    backend = _Exec(_routes({"readlink": "6-link", "cat": manifest}))
    plan = catalog.Deployment(attrs=[], built=[built])
    with pytest.raises(Abort, match="not replaced by their served store paths"):
        catalog.verify_profile_advanced(Device("h", backend=backend), plan, cycle)


def test_verify_profile_advanced_aborts_on_non_json_manifest() -> None:
    cycle = _cycle(generation_before=5, installed_before={"core"})
    built = Built("core", "1.0", Attr(".#x"), store_path=StorePath("/nix/store/abc-core"))
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
            Built("core", "1.0", Attr(".#core^out"), store_path=StorePath("/nix/store/core")),
            Built(
                "widget-weather",
                "0.1.0",
                Attr(".#w^out"),
                store_path=StorePath("/nix/store/weather"),
            ),
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
            _HOST_CLI_ATTR: _stub_host_cli(tmp_path),
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


def test_assemble_rig_signs_variants_before_writing_the_feed(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path)
    nix = _e2e_nix(tmp_path)
    catalog.build_e2e_artifacts(nix, run)
    workdir = tmp_path / "work"
    catalog.assemble_rig(nix, run, workdir=workdir, base_url="http://10.0.0.9:8083")
    feed = json.loads((workdir / "serve" / rig.FEED_NAME).read_text())
    assert all(e["signature"] == "sysupgrade-e2e-1:STUBSIG" for e in feed["entries"])
    assert run.variant_a is not None
    assert run.variant_a.signature == "sysupgrade-e2e-1:STUBSIG"
    assert run.rig is not None
    assert run.rig.secret == workdir / "cache-key.secret"
    assert run.rig.serve_root == workdir / "serve"
    assert run.rig.cache == workdir / "serve" / "cache"
    assert run.rig.host_cli == str(tmp_path / "host-cli")


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


def test_capture_server_registry_records_the_present_file() -> None:
    content = b"BASE64DATA"
    exc = _Exec(
        _routes(
            {
                "test -f /etc/nix-upgrade/servers.json": "present",
                "base64 < /etc/nix-upgrade/servers.json": base64.b64encode(content).decode(),
                "wc -c < /etc/nix-upgrade/servers.json": str(len(content)),
                "sha256sum /etc/nix-upgrade/servers.json": hashlib.sha256(content).hexdigest(),
            }
        )
    )
    plan = catalog.Provisioning()
    catalog.capture_server_registry(Device("h", backend=exc), plan)
    assert plan.servers_snapshot is not None
    assert plan.servers_snapshot.contents == content


def test_capture_server_registry_records_absence() -> None:
    exc = _Exec(_routes({}))
    plan = catalog.Provisioning()
    catalog.capture_server_registry(Device("h", backend=exc), plan)
    assert plan.servers_snapshot is not None
    assert not plan.servers_snapshot.present


def test_restore_server_registry_rewrites_the_original_content() -> None:
    exc = _Exec(_routes({}))
    plan = catalog.Provisioning(
        servers_snapshot=catalog.FileSnapshot(catalog.SERVERS_JSON, None, b"BASE64")
    )
    catalog.restore_server_registry(Device("h", backend=exc), plan)
    cmd = exc.runs[-1][-1]
    assert "echo QkFTRTY0 | base64 -d" in cmd
    assert "mv /etc/nix-upgrade/servers.json.tmp /etc/nix-upgrade/servers.json" in cmd


def test_restore_server_registry_removes_what_was_absent_before() -> None:
    exc = _Exec(_routes({}))
    plan = catalog.Provisioning(servers_snapshot=catalog.FileSnapshot(catalog.SERVERS_JSON, None))
    catalog.restore_server_registry(Device("h", backend=exc), plan)
    assert [argv[-1] for argv in exc.runs] == ["rm -f /etc/nix-upgrade/servers.json"]


def test_restore_server_registry_without_capture_aborts() -> None:
    with pytest.raises(Abort, match="without a prior capture"):
        catalog.restore_server_registry(
            Device("h", backend=_Exec(_routes({}))), catalog.Provisioning()
        )


def test_register_rig_tampered_uses_the_wrong_key_everywhere(tmp_path: Path) -> None:
    backend = _Exec(_routes({"cat /etc/nix/nix.conf": "substituters ...\nWRONGKEY\n"}))
    run = _rigged_run(tmp_path)  # populated E2eRig, as register_rig's own tests use
    catalog.register_rig_tampered(
        Device("h", backend=backend), run, wrong_public_key="sysupgrade-e2e-1:WRONGKEY"
    )
    joined = [" ".join(argv) for argv in backend.runs]
    registration = next(c for c in joined if "register-server" in c)
    assert "sysupgrade-e2e-1:WRONGKEY" in registration
    assert run.rig is not None
    assert run.rig.cache_public_key not in registration


def test_register_rig_tampered_aborts_when_the_good_key_survives(tmp_path: Path) -> None:
    run = _rigged_run(tmp_path)
    assert run.rig is not None
    good = run.rig.cache_public_key
    backend = _Exec(_routes({"cat /etc/nix/nix.conf": f"trusted keys: {good}\n"}))
    with pytest.raises(Abort, match="still trusted"):
        catalog.register_rig_tampered(
            Device("h", backend=backend), run, wrong_public_key="sysupgrade-e2e-1:WRONGKEY"
        )


def test_witness_and_remnant_stages_round_trip() -> None:
    backend = _Exec(_routes({}))
    dev = Device("h", backend=backend)
    catalog.plant_store_witness(dev)
    catalog.plant_store_remnants(dev)
    catalog.delete_store_db(dev)
    joined = [" ".join(argv) for argv in backend.runs]
    assert any(".bdk601-witness" in c for c in joined)
    assert any("nix.tmp" in c and "nix.wiped" in c for c in joined)
    assert any("db.sqlite" in c for c in joined)
    gone = Device("h", backend=_Exec(_routes({})))  # empty read = absent
    catalog.require_witness_gone(gone)
    catalog.require_remnants_gone(gone)
    present = Device("h", backend=_Exec(_routes({"witness": "yes", "nix.tmp": "yes"})))
    with pytest.raises(Abort):
        catalog.require_witness_gone(present)


def test_shm_stages_probe_upload_and_sweep(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_cp)  # placeholder responder; the real one closes over `backend`

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1] if argv and argv[0] == "ssh" else " ".join(argv)
        # standard Linux: a real /dev/shm dir with its own dedicated tmpfs mount
        if "readlink -f /dev/shm" in cmd:
            return _cp(argv, "/dev/shm")
        if "cat /proc/mounts" in cmd:
            return _cp(argv, "tmpfs /dev/shm tmpfs rw 0 0")
        if "sha256sum" in cmd:
            # stateful: absent before the upload streamed, intact after —
            # a fake that reports the digest up front would let done_if
            # skip the push and prove nothing. _remote_sha pipes the
            # sha256sum line through `cut -d' ' -f1` on the device, so it
            # returns the bare digest — model that, not the raw two-column line.
            digest = image.sha256 if backend.streams else "0" * 64
            return _cp(argv, digest)
        return _cp(argv)

    backend._respond = respond
    dev = Device("h", backend=backend)
    catalog.require_shm_tmpfs(dev)
    catalog.upload_firmware_shm(dev, image)
    argv, data = backend.streams[0]
    assert f"/dev/shm/{image.path.name}" in argv[-1]
    assert data == image.path.read_bytes()
    catalog.sweep_shm_upload(dev, image)
    assert any("rm -f" in " ".join(a) and "/dev/shm/" in " ".join(a) for a in backend.runs)


def test_shm_probe_passes_on_openwrt_symlinked_shm() -> None:
    """OpenWRT reality: /dev/shm is a symlink to /tmp/shm and NO dedicated
    shm mount exists — the probe must judge the mount containing the
    RESOLVED path, component-wise: the tmpfs on /tmp contains /tmp/shm,
    while the /tmp/sh decoy mount is only a string prefix and must not win."""
    backend = _Exec(
        _routes(
            {
                "readlink -f /dev/shm": "/tmp/shm",
                "cat /proc/mounts": (
                    "/dev/root / ext4 rw 0 0\n"
                    "loop /tmp/sh ext4 rw 0 0\n"
                    "tmpfs /tmp tmpfs rw,noatime,size=102400k 0 0\n"
                ),
            }
        )
    )
    catalog.require_shm_tmpfs(Device("h", backend=backend))


def test_shm_probe_fails_when_resolved_path_is_not_tmpfs_backed() -> None:
    backend = _Exec(
        _routes(
            {
                "readlink -f /dev/shm": "/tmp/shm",
                "cat /proc/mounts": "/dev/root / ext4 rw 0 0\n/dev/mmcblk0p4 /tmp ext4 rw 0 0\n",
            }
        )
    )
    with pytest.raises(Abort, match="/tmp/shm"):
        catalog.require_shm_tmpfs(Device("h", backend=backend))


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


def _cleardown_routes(
    *, holders: str = "", orchestrator_lingers: bool = False, mount_layers: int = 1
) -> _Respond:
    state = {"umounts": 0, "deleted": False}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "service delete" in cmd:
            state["deleted"] = True
        if "umount" in cmd:
            state["umounts"] += 1
        gone = state["deleted"] and not orchestrator_lingers
        outputs = {
            "service list": "" if gone else '{"bmc-nix-service-orchestrator":{}}',
            "/proc/mounts": "" if state["umounts"] >= mount_layers else "/dev/x /nix ext4 rw 0 0",
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


def test_cleardown_peels_stacked_nix_mounts() -> None:
    exc = _Exec(_cleardown_routes(mount_layers=3))
    catalog.clear_nix_store(Device("h", backend=exc), assume_yes=True)
    cmds = [argv[-1] for argv in exc.runs]
    assert cmds.count("umount /nix") == 3
    assert "rm -rf /mnt/data/nix" in cmds


def test_cleardown_aborts_when_the_mount_never_clears() -> None:
    exc = _Exec(_cleardown_routes(mount_layers=99))
    with pytest.raises(Abort, match="still mounted"):
        catalog.clear_nix_store(Device("h", backend=exc), assume_yes=True)
    assert not any("rm -rf" in argv[-1] for argv in exc.runs)


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


def _ticking_clock() -> Callable[[], float]:
    ticks = iter(range(0, 100_000))
    return lambda: float(next(ticks))


def test_quiesce_nix_stops_services_and_unmounts_without_deleting() -> None:
    exc = _Exec(_cleardown_routes())
    catalog.quiesce_nix(Device("h", backend=exc), sleep=lambda _s: None, clock=_ticking_clock())
    joined = [argv[-1] for argv in exc.runs]
    assert not any("rm -rf" in c for c in joined)  # quiesce never deletes
    assert any("umount /nix" in c for c in joined)


def _b64(content: str) -> str:
    return base64.b64encode(content.encode()).decode()


def _preservation_routes(servers_json: Callable[[], str | None]) -> _Respond:
    """nix.conf intact; the servers.json probes answer per call — None is
    absent, a str is the file's content (served base64 to the capture)."""

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "/etc/nix/nix.conf" in cmd:
            return _cp(argv, "experimental-features = nix-command")
        if "if [ -e" in cmd and "servers.json" in cmd:
            content = servers_json()
            return _cp(argv, "ABSENT" if content is None else f"PRESENT\n{_b64(content)}")
        if "servers.json ]" in cmd:
            return _cp(argv, "" if servers_json() is None else "yes")
        return _cp(argv)

    return respond


def _recorded_state(content: str) -> "catalog.FaultsState":
    state = catalog.FaultsState()
    state.servers_json_before = _b64(content)
    return state


def test_record_servers_json_snapshots_the_registry_bytes() -> None:
    """The D5 proof is a byte comparison — the record stage must capture
    the exact pre-flash content, not just note the file exists."""
    state = catalog.FaultsState()
    exc = _Exec(_preservation_routes(lambda: '{"servers": "rig"}'))
    catalog.record_servers_json(Device("h", backend=exc), state)
    assert state.servers_json_before == _b64('{"servers": "rig"}')


def test_record_servers_json_aborts_when_registration_left_nothing() -> None:
    """An absent registry before the D5 flash means the register stage did
    not do its job — a preservation verdict on it would be meaningless."""
    state = catalog.FaultsState()
    exc = _Exec(_preservation_routes(lambda: None))
    with pytest.raises(Abort, match="registration did not write it"):
        catalog.record_servers_json(Device("h", backend=exc), state)


def test_preservation_policy_observes_when_not_asserting(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """--no-servers-json-preserved is the legacy escape hatch for images
    predating the conffile registration: post-flash state is reported,
    never asserted — either polarity passes on a single probe."""
    exc = _Exec(_preservation_routes(lambda: None))
    catalog.require_preservation_policy(
        Device("h", backend=exc), catalog.FaultsState(), servers_json_preserved=False
    )
    assert "gone (not asserted" in capsys.readouterr().out

    polls = {"n": 0}

    def probed() -> str | None:
        polls["n"] += 1
        return "{}"

    exc = _Exec(_preservation_routes(probed))
    catalog.require_preservation_policy(
        Device("h", backend=exc), catalog.FaultsState(), servers_json_preserved=False
    )
    assert polls["n"] == 1  # single-shot: with no expected state there is nothing to poll for
    assert "present (not asserted" in capsys.readouterr().out


def test_preservation_policy_passes_immediately_on_identical_bytes() -> None:
    """The default contract: the post-flash file matches the pre-flash
    snapshot byte for byte — accepted on the first observation without
    burning any waiting time."""
    polls = {"n": 0}

    def probed() -> str | None:
        polls["n"] += 1
        return '{"servers": "rig"}'

    exc = _Exec(_preservation_routes(probed))
    sleeps: list[float] = []
    catalog.require_preservation_policy(
        Device("h", backend=exc),
        _recorded_state('{"servers": "rig"}'),
        servers_json_preserved=True,
        sleep=sleeps.append,
        clock=_ticking_clock(),
    )
    assert polls["n"] == 2  # one presence probe, one content capture
    assert sleeps == []  # no polling wait was consumed


def test_preservation_policy_aborts_when_the_bytes_changed() -> None:
    """Existence alone proves nothing — a flash that replaced the registry
    with different defaults must fail the preservation assert (the review
    finding: the old stage passed on any present file)."""
    exc = _Exec(_preservation_routes(lambda: '{"servers": "factory-default"}'))
    with pytest.raises(Abort, match="contents changed across the flash"):
        catalog.require_preservation_policy(
            Device("h", backend=exc),
            _recorded_state('{"servers": "rig"}'),
            servers_json_preserved=True,
            sleep=lambda _s: None,
            clock=_ticking_clock(),
        )


def test_preservation_policy_aborts_when_a_preserved_file_never_appears() -> None:
    """preserved=True with the file genuinely wiped: absent-then-restored is
    not an expected transition, so the poll ends only via the full timeout,
    aborting with the 'missing' polarity."""
    exc = _Exec(_preservation_routes(lambda: None))
    with pytest.raises(Abort, match=r"still missing \d+s after the flash"):
        catalog.require_preservation_policy(
            Device("h", backend=exc),
            _recorded_state("{}"),
            servers_json_preserved=True,
            timeout=10,
            sleep=lambda _s: None,
            clock=_ticking_clock(),
        )


def test_preservation_policy_refuses_to_assert_without_a_snapshot() -> None:
    """A driver that skipped the record stage cannot get a preservation
    verdict — that is a harness wiring bug, not a device failure."""
    exc = _Exec(_preservation_routes(lambda: "{}"))
    with pytest.raises(RuntimeError, match=r"BUG: servers\.json was not recorded"):
        catalog.require_preservation_policy(
            Device("h", backend=exc), catalog.FaultsState(), servers_json_preserved=True
        )


_MOUNTINFO = (
    "21 1 179:4 / /mnt/data rw,relatime shared:10 - ext4 /dev/mmcblk0p4 rw\n"
    "30 21 179:4 /nix / /nix rw - ext4 /dev/mmcblk0p4 rw\n"
)
_MOUNTINFO_RELEASED = "17 1 0:5 / /dev rw - devtmpfs devtmpfs rw\n"


def test_release_data_partition_records_identity_and_asserts_by_majmin(tmp_path: Path) -> None:
    reads = iter([_MOUNTINFO, _MOUNTINFO_RELEASED])

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "mountinfo" in cmd:
            return _cp(argv, next(reads))
        if cmd.startswith("blkid"):
            return _cp(argv, '/dev/mmcblk0p4: UUID="1111-2222" TYPE="ext4"')
        return _cp(argv)

    backend = _Exec(respond)
    state = catalog.FaultsState()
    catalog.release_data_partition(Device("h", backend=backend), state)
    assert state.partition == catalog.DataPartition(
        device="/dev/mmcblk0p4", majmin="179:4", uuid="1111-2222"
    )
    assert any("umount /mnt/data" in " ".join(a) for a in backend.runs)


def test_release_data_partition_aborts_when_a_mount_remains_on_the_device() -> None:
    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "mountinfo" in cmd:
            return _cp(argv, _MOUNTINFO)  # never changes: umount "didn't work"
        if cmd.startswith("blkid"):
            return _cp(argv, '/dev/mmcblk0p4: UUID="1111-2222" TYPE="ext4"')
        return _cp(argv)

    with pytest.raises(Abort, match="179:4"):
        catalog.release_data_partition(Device("h", backend=_Exec(respond)), catalog.FaultsState())


def _debugfs_out(tmp_path: Path) -> Path:
    out = tmp_path / "debugfs-out"
    (out / "sbin").mkdir(parents=True, exist_ok=True)
    (out / "sbin" / "debugfs").write_bytes(b"elf")
    return out


def test_build_and_push_debugfs_ships_the_static_binary(tmp_path: Path) -> None:
    plan = catalog.Provisioning()
    nix = _FakeNix({catalog._DEBUGFS_ATTR: _debugfs_out(tmp_path)})
    catalog.build_debugfs(nix, plan)
    assert plan.debugfs is not None
    assert plan.debugfs.name == "debugfs"

    backend = _Exec(_routes({}))
    catalog.push_debugfs(Device("h", backend=backend), plan)
    # the binary is streamed to the tmpfs path, then made executable there
    assert any(catalog._REMOTE_DEBUGFS in " ".join(argv) for argv, _ in backend.streams)
    assert any(
        "chmod +x" in " ".join(argv) and catalog._REMOTE_DEBUGFS in " ".join(argv)
        for argv in backend.runs
    )


def test_corrupt_partition_metadata_runs_the_pushed_debugfs_recipe() -> None:
    backend = _Exec(_routes({}))
    state = catalog.FaultsState()
    state.partition = catalog.DataPartition("/dev/mmcblk0p4", "179:4", "1111-2222")
    catalog.corrupt_partition_metadata(Device("h", backend=backend), state)
    joined = [" ".join(argv) for argv in backend.runs]
    # each recipe command runs through the pushed static binary, not a bare
    # `debugfs` — OpenWRT ships none, so a literal `debugfs` would fail on device
    for command in catalog._B2_DEBUGFS_COMMANDS:
        assert any(command in c and catalog._REMOTE_DEBUGFS in c for c in joined)


def test_fs_uuid_assertions_read_blkid() -> None:
    backend = _Exec(_routes({"blkid": '/dev/mmcblk0p4: UUID="3333-4444" TYPE="ext4"'}))
    state = catalog.FaultsState()
    state.partition = catalog.DataPartition("/dev/mmcblk0p4", "179:4", "1111-2222")
    catalog.require_fs_uuid_changed(Device("h", backend=backend), state)
    with pytest.raises(Abort):
        catalog.require_fs_uuid_unchanged(Device("h", backend=backend), state)


def test_fs_uuid_assertions_skip_under_dry_run() -> None:
    """Dry-run only logs the mkfs/repair mutations, so the real UUID cannot
    have changed — probing it would abort every --dry-run B1 with a
    misleading 'mkfs did not run'. The stages must not even need the
    partition record: release_data_partition never ran."""
    backend = _Exec(_routes({}))
    state = catalog.FaultsState()  # no partition recorded
    token = dry_run.set(True)
    try:
        catalog.require_fs_uuid_changed(Device("h", backend=backend), state)
        catalog.require_fs_uuid_unchanged(Device("h", backend=backend), state)
    finally:
        dry_run.reset(token)
    assert backend.runs == []  # no blkid probe reached the device


def test_flash_e2e_never_skips_on_matching_version(tmp_path: Path) -> None:
    image = _image(tmp_path)
    exc = _Exec(_routes({"bos_version": image.version}))
    catalog.flash_e2e(Device("h", backend=exc), image, assume_yes=True)
    assert any(argv[-1] == f"sysupgrade {shlex.quote(image.remote_path)}" for argv in exc.runs)


def test_flash_e2e_aborts_when_declined(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(catalog.console, "confirm", lambda _q: False)
    with pytest.raises(Abort, match="--yes"):
        catalog.flash_e2e(Device("h", backend=_Exec(_routes({}))), _image(tmp_path))


def _abort_backend(returncode: int, output: str) -> _Exec:
    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if cmd.startswith("sysupgrade"):
            raise subprocess.CalledProcessError(returncode, argv, output=output, stderr="")
        if "bos_version" in cmd:
            return _cp(argv, "2026-06-14-x")
        return _cp(argv)

    return _Exec(respond)


def test_flash_expect_abort_passes_on_matching_remote_failure(tmp_path: Path) -> None:
    image = _image(tmp_path)
    state = catalog.FaultsState()
    dev = Device("h", backend=_abort_backend(1, "init tarball signature verification failed"))
    catalog.flash_expect_abort(
        dev, image, expect="signature verification failed", state=state, assume_yes=True
    )
    assert "signature verification failed" in (state.flash_output or "")


def test_flash_expect_abort_rejects_clean_exit(tmp_path: Path) -> None:
    image = _image(tmp_path)
    backend = _Exec(_routes({"bos_version": "2026-06-14-x"}))
    with pytest.raises(Abort, match="did not fire"):
        catalog.flash_expect_abort(
            Device("h", backend=backend),
            image,
            expect="whatever",
            state=catalog.FaultsState(),
            assume_yes=True,
        )


def test_flash_expect_abort_rejects_session_death(tmp_path: Path) -> None:
    image = _image(tmp_path)
    dev = Device("h", backend=_abort_backend(255, "flashing..."))
    with pytest.raises(Abort, match="session"):
        catalog.flash_expect_abort(
            dev, image, expect="x", state=catalog.FaultsState(), assume_yes=True
        )


def test_flash_expect_abort_rejects_wrong_message(tmp_path: Path) -> None:
    image = _image(tmp_path)
    dev = Device("h", backend=_abort_backend(1, "some other failure"))
    with pytest.raises(Abort, match="does not mention"):
        catalog.flash_expect_abort(
            dev,
            image,
            expect=("signature", "stalled"),
            state=catalog.FaultsState(),
            assume_yes=True,
        )


def test_upgrade_state_round_trip_detects_a_new_next_marker() -> None:
    routes = {
        "readlink -f /nix/var/nix/gcroots/profiles/bmc/current": "/nix/store/gen-3",
        ".sysupgrade-e2e-marker": "yes",
        "next.": "",
        "boot_id": "boot-before",
    }
    state = catalog.FaultsState()
    catalog.record_upgrade_state(Device("h", backend=_Exec(_routes(routes))), state)
    assert state.upgrade_state == catalog.UpgradeState(
        current="/nix/store/gen-3", marker_present=True, next_markers=[], boot_id="boot-before"
    )
    catalog.require_upgrade_state_untouched(Device("h", backend=_Exec(_routes(routes))), state)
    routes["next."] = "next.2026-07-15-x"
    with pytest.raises(Abort, match="next"):
        catalog.require_upgrade_state_untouched(Device("h", backend=_Exec(_routes(routes))), state)


def test_require_rebooted_needs_a_fresh_boot_id() -> None:
    """The same-version re-flashes (C6/D1/D4) verify an unchanged version
    and an untouched store — checks a sysupgrade that exits zero WITHOUT
    flashing or rebooting also satisfies. The boot-id comparison is the
    only assertion in the chain such a no-op cannot pass."""
    routes = {
        "readlink -f /nix/var/nix/gcroots/profiles/bmc/current": "/nix/store/gen-3",
        "boot_id": "boot-before",
    }
    state = catalog.FaultsState()
    catalog.record_upgrade_state(Device("h", backend=_Exec(_routes(routes))), state)
    with pytest.raises(Abort, match="without rebooting"):
        catalog.require_rebooted(Device("h", backend=_Exec(_routes(routes))), state)
    routes["boot_id"] = "boot-after"
    catalog.require_rebooted(Device("h", backend=_Exec(_routes(routes))), state)


def test_require_staged_once_counts_the_staging_lines() -> None:
    state = catalog.FaultsState()
    init_token, upgrade_token = catalog._STAGING_TOKENS
    state.flash_output = f"{init_token}\nother output\n"
    catalog.require_staged_once(state)
    # one init + one upgrade line would mean staging ran twice
    state.flash_output = f"{init_token}\n{upgrade_token}\n"
    with pytest.raises(Abort, match="once"):
        catalog.require_staged_once(state)


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


def test_trust_image_keys_extracts_the_rootfs_keys_onto_the_device(tmp_path: Path) -> None:
    run = _e2e_run(tmp_path)
    exc = _Exec(_routes({}))
    catalog.trust_image_keys(Device("h", backend=exc), run.image_a)
    (cmd,) = [argv[-1] for argv in exc.runs]
    assert f"tar -xf /tmp/{run.image_a.path.name}" in cmd
    assert f'unsquashfs -q -n -d "$d/r" "$d/{_TOP}/rootfs.img" etc/opkg/keys' in cmd
    assert 'cp "$d/r/etc/opkg/keys/"* /etc/opkg/keys/' in cmd


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


def _e2e_full_routes(sha_a: str, sha_b: str) -> _Respond:
    board = json.dumps({"board_name": "b", "release": {"target": _TARGET}})
    state = {"flashes": 0, "mounted": True, "marker": False}

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if cmd.startswith("sysupgrade "):
            state["flashes"] += 1
            state["mounted"] = True
            return _cp(argv)
        if cmd.startswith("touch ") and ".sysupgrade-e2e-marker" in cmd:
            state["marker"] = True
            return _cp(argv)
        if "umount" in cmd:
            state["mounted"] = False
            return _cp(argv)
        if cmd.startswith("ls") and ("rc.d" in cmd or "init.d" in cmd):
            return _cp(argv, "S20bar" if "rc.d" in cmd else "bar")
        outputs = {
            "if [ -e /etc/nix-upgrade/servers.json ]": "ABSENT",
            "if [ -e /etc/nix/nix.conf ]": "ABSENT",
            "ubus call system board": board,
            "bos_version": ["old", "va", "vb"][state["flashes"]],
            "MemAvailable": "524288",
            "sha256sum": sha_a if "a.tar" in cmd else sha_b,
            "dd bs=64": "64",
            "service list": "",
            "[ -f /mnt/data/nix/.sysupgrade-e2e-marker ]": "yes" if state["marker"] else "",
            "/proc/mounts": "/dev/x /nix ext4 rw 0 0" if state["mounted"] else "",
            "readlink -f": _GEN if state["flashes"] < 2 else _GEN.replace("/5", "/6"),
            "manifest": "yes",
            "gcroots/profiles/bmc 2>/dev/null": "1-link\ncurrent",
            " status ": "ok",
            "[ -d /mnt/data/nix ]": "yes",
            "/proc/[0-9]*": "",
            "-ef": "yes",
        }
        for key, value in outputs.items():
            if key in cmd:
                return _cp(argv, value)
        return _cp(argv)

    return respond


def _e2e_args(
    tmp_path: Path,
    *,
    scenario: Literal["init", "upgrade", "full"] = "full",
    dry_run_flag: bool = False,
) -> tuple[E2eSysupgrade, Image, Image]:
    a = _e2e_image(tmp_path, name="a.tar", version="va")
    b = _e2e_image(tmp_path, name="b.tar", version="vb")
    args = E2eSysupgrade(
        device="127.0.0.1",
        image_a=a.path,
        image_b=b.path,
        serve_ip="127.0.0.1",
        scenario=scenario,
        yes=True,
        dry_run=dry_run_flag,
    )
    return args, a, b


def _local_server(root: Path) -> rig.RigServer:
    return rig.RigServer(root, port=0, bind_ip="127.0.0.1")


def test_e2e_procedure_full_run_orders_the_scenarios(tmp_path: Path) -> None:
    args, a, b = _e2e_args(tmp_path)
    exc = _Exec(_e2e_full_routes(a.sha256, b.sha256))
    dev = Device("127.0.0.1", backend=exc)
    args.run(
        dev=dev,
        backend=_e2e_nix(tmp_path),
        make_device=lambda _h: dev,
        make_server=_local_server,
    )
    cmds = [argv[-1] for argv in exc.runs]
    flashes = [i for i, c in enumerate(cmds) if c.startswith("sysupgrade ")]
    registers = [i for i, c in enumerate(cmds) if "register-server" in c]
    assert len(flashes) == 2
    assert len(registers) == 2, "registration must re-run before EACH flash"
    assert registers[0] < flashes[0] < registers[1] < flashes[1]
    memory_gates = [i for i, c in enumerate(cmds) if "MemAvailable" in c]
    compositor_stops = [i for i, c in enumerate(cmds) if c == "service bmc-compositor stop"]
    assert len(memory_gates) == 2
    assert compositor_stops[0] < memory_gates[0]
    assert any(flashes[0] < stop < memory_gates[1] for stop in compositor_stops)
    umount = cmds.index("umount /nix")
    sha_checks_a = [i for i, c in enumerate(cmds) if "sha256sum" in c and "a.tar" in c]
    assert sha_checks_a and min(sha_checks_a) < umount < flashes[0]
    trusts = [i for i, c in enumerate(cmds) if "unsquashfs" in c]
    assert len(trusts) == 2, "each flash must trust the incoming image's keys first"
    assert min(sha_checks_a) < trusts[0] < umount and trusts[1] < flashes[1]
    marker = next(i for i, c in enumerate(cmds) if c.startswith("touch "))
    readlink_b = next(i for i, c in enumerate(cmds) if "readlink -f" in c and i > registers[1])
    assert registers[1] < marker < readlink_b < flashes[1]
    wgets = [i for i, c in enumerate(cmds) if "wget" in c]
    destructive = [i for i, c in enumerate(cmds) if "rm -rf" in c or c == "umount /nix"]
    assert wgets and destructive and max(wgets) < min(destructive)
    sweep = [c for c in cmds if c.startswith("rm -f ")]
    assert "rm -f /mnt/data/nix/.sysupgrade-e2e-marker" in sweep
    assert "rm -f /etc/nix-upgrade/servers.json" in sweep
    assert "rm -f /tmp/a.tar /tmp/b.tar" in sweep


def test_e2e_cleanup_restarts_the_compositor_after_a_failure(tmp_path: Path) -> None:
    """Both scenarios stop the compositor before their flash; a run that
    dies before rebooting must hand the Deck back with its UI, not dark."""
    args, a, b = _e2e_args(tmp_path)
    inner = _e2e_full_routes(a.sha256, b.sha256)

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        if "MemAvailable" in argv[-1]:
            return _cp(argv, "0")
        return inner(argv)

    exc = _Exec(respond)
    dev = Device("127.0.0.1", backend=exc)
    with pytest.raises(Abort, match="free RAM"):
        args.run(
            dev=dev,
            backend=_e2e_nix(tmp_path),
            make_device=lambda _h: dev,
            make_server=_local_server,
        )
    cmds = [argv[-1] for argv in exc.runs]
    assert cmds.index("service bmc-compositor start") > cmds.index("service bmc-compositor stop")


def test_e2e_procedure_routes_destructive_stages_through_the_pinned_device(
    tmp_path: Path,
) -> None:
    args, a, b = _e2e_args(tmp_path)
    respond = _e2e_full_routes(a.sha256, b.sha256)
    exc, pinned_exc = _Exec(respond), _Exec(respond)
    dev = Device("localhost", backend=exc)  # resolves to 127.0.0.1, so pinning re-addresses
    pinned = Device("127.0.0.1", backend=pinned_exc)
    args.run(
        dev=dev,
        backend=_e2e_nix(tmp_path),
        make_device=lambda _h: pinned,
        make_server=_local_server,
    )
    cmds = [argv[-1] for argv in exc.runs]
    pinned_cmds = [argv[-1] for argv in pinned_exc.runs]
    assert exc.streams and not pinned_exc.streams  # uploads ride the name-addressed device
    assert not any(c.startswith("sysupgrade ") or "rm -rf" in c or "umount" in c for c in cmds)
    assert len([c for c in pinned_cmds if c.startswith("sysupgrade ")]) == 2
    assert any("rm -rf /mnt/data/nix" in c for c in pinned_cmds)
    # key trust mutates /etc/opkg/keys, so it must ride the pinned connection too
    assert len([c for c in pinned_cmds if "unsquashfs" in c]) == 2
    sha_ties = [c for c in pinned_cmds if "sha256sum" in c]
    assert len(sha_ties) == 2  # the identity tie re-probes each upload via the pinned address


def test_e2e_procedure_dry_run_touches_nothing(tmp_path: Path) -> None:
    args, a, b = _e2e_args(tmp_path, dry_run_flag=True)
    exc = _Exec(_e2e_full_routes(a.sha256, b.sha256))
    dev = Device("127.0.0.1", backend=exc)
    token = dry_run.set(False)  # the procedure sets it from its own flag
    try:
        args.run(
            dev=dev,
            backend=_e2e_nix(tmp_path),
            make_device=lambda _h: dev,
            make_server=_local_server,
        )
    finally:
        dry_run.reset(token)
    cmds = [argv[-1] for argv in exc.runs]
    assert not any(c.startswith("sysupgrade ") for c in cmds)
    assert not any("rm -rf" in c or "umount" in c for c in cmds)
    assert not exc.streams  # no uploads happened


def test_e2e_procedure_init_scenario_skips_upgrade_stages(tmp_path: Path) -> None:
    args, a, b = _e2e_args(tmp_path, scenario="init")
    exc = _Exec(_e2e_full_routes(a.sha256, b.sha256))
    dev = Device("127.0.0.1", backend=exc)
    args.run(
        dev=dev,
        backend=_e2e_nix(tmp_path),
        make_device=lambda _h: dev,
        make_server=_local_server,
    )
    cmds = [argv[-1] for argv in exc.runs]
    assert len([c for c in cmds if c.startswith("sysupgrade ")]) == 1
    assert not any("cli-b" in c for c in cmds)  # no bump-absence probe ran
    assert not any(c.startswith("touch ") for c in cmds)  # no marker dropped


def test_e2e_upgrade_scenario_aborts_readonly_on_uninitialized_store(tmp_path: Path) -> None:
    args, a, b = _e2e_args(tmp_path, scenario="upgrade")
    base = _e2e_full_routes(a.sha256, b.sha256)

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "bos_version" in cmd:
            return _cp(argv, "va")
        if "[ -d /mnt/data/nix ]" in cmd or "readlink -f" in cmd:
            return _cp(argv)
        return base(argv)

    exc = _Exec(respond)
    dev = Device("127.0.0.1", backend=exc)
    with pytest.raises(Abort, match="not initialized"):
        args.run(
            dev=dev,
            backend=_e2e_nix(tmp_path),
            make_device=lambda _h: dev,
            make_server=_local_server,
        )
    cmds = [argv[-1] for argv in exc.runs]
    assert not exc.streams
    # The registry snapshot probes are read-only and allowed.
    assert not any(
        "register-server" in c
        or "chmod" in c
        or f"> {catalog.SERVERS_JSON}" in c
        or f"rm -f {catalog.SERVERS_JSON}" in c
        or f"mv {catalog.SERVERS_JSON}" in c
        for c in cmds
    )
    assert not any(c.startswith("touch ") or c.startswith("sysupgrade ") for c in cmds)
    assert not any("rm -f" in c for c in cmds)


def test_e2e_upgrade_scenario_aborts_readonly_on_existing_bump(tmp_path: Path) -> None:
    args, a, b = _e2e_args(tmp_path, scenario="upgrade")
    base = _e2e_full_routes(a.sha256, b.sha256)

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        cmd = argv[-1]
        if "bos_version" in cmd:
            return _cp(argv, "va")
        if cmd.startswith("if [ -e "):
            return base(argv)  # the read-only capture probes, routed normally
        if "[ -e " in cmd:
            return _cp(argv, "yes")
        return base(argv)

    exc = _Exec(respond)
    dev = Device("127.0.0.1", backend=exc)
    with pytest.raises(Abort, match="already"):
        args.run(
            dev=dev,
            backend=_e2e_nix(tmp_path),
            make_device=lambda _h: dev,
            make_server=_local_server,
        )
    cmds = [argv[-1] for argv in exc.runs]
    assert not exc.streams
    # The registry snapshot probes are read-only and allowed.
    assert not any(
        "register-server" in c
        or "chmod" in c
        or f"> {catalog.SERVERS_JSON}" in c
        or f"rm -f {catalog.SERVERS_JSON}" in c
        or f"mv {catalog.SERVERS_JSON}" in c
        for c in cmds
    )
    assert not any(c.startswith("touch ") or c.startswith("sysupgrade ") for c in cmds)
    assert not any("rm -f" in c for c in cmds)


# ── e2e firmware upgrade ─────────────────────────────────────────────────────────────────


def _firmware_cycle(tmp_path: Path) -> catalog.FirmwareCycle:
    return catalog.FirmwareCycle(
        password="",
        index_port=8082,
        stream_deadline=600,
        snapshot_dir=tmp_path,
        device_identity="88:a6:ef:d1:17:6e",
    )


def test_preflight_versions_accepts_strictly_newer_image(tmp_path: Path) -> None:
    running = "2025-06-15-0-acde0123-25.06"
    image = _image(tmp_path, version="2025-07-01-0-0badc0de-25.07")
    cycle = _firmware_cycle(tmp_path)
    catalog.preflight_versions(
        Device("h", backend=_Exec(_routes({"cat /etc/bos_version": running}))), image, cycle
    )
    assert cycle.running_version is not None
    assert cycle.running_version.canonical == running
    assert cycle.image_version is not None
    assert cycle.image_version.canonical == image.version


@pytest.mark.parametrize(
    ("running", "image"),
    [
        ("2025-06-15-0-acde0123-25.06", "2025-07-01-0-0badc0de-25.06"),
        ("2025-06-15-0-acde0123-25.06", "2025-05-01-0-0badc0de-25.05"),
    ],
)
def test_preflight_versions_accepts_equal_and_older_release(
    tmp_path: Path, running: str, image: str
) -> None:
    cycle = _firmware_cycle(tmp_path)
    catalog.preflight_versions(
        Device("h", backend=_Exec(_routes({"cat /etc/bos_version": running}))),
        _image(tmp_path, version=image),
        cycle,
    )
    assert cycle.running_version is not None
    assert cycle.running_version.canonical == running
    assert cycle.image_version is not None
    assert cycle.image_version.canonical == image


def test_anchored_version_decrements_image_release() -> None:
    running = catalog.parse_bos_version("2026-07-15-3-8a7eb005-26.08-plus-nightly")

    anchored = catalog.anchored_version(
        running, catalog.parse_bos_version("2026-08-01-0-00000001-26.08")
    )
    assert anchored.canonical == "2026-07-15-3-8a7eb005-26.07-plus-nightly"

    borrowed = catalog.anchored_version(
        running, catalog.parse_bos_version("2027-01-05-0-00000002-27.01")
    )
    assert borrowed.version == catalog.VersionName(26, 12, None)

    patched = catalog.anchored_version(
        running, catalog.parse_bos_version("2026-08-01-0-00000001-26.08.2")
    )
    assert patched.version == catalog.VersionName(26, 7, None)

    with pytest.raises(ValueError, match=r"0\.01"):
        catalog.anchored_version(running, replace(running, version=catalog.VersionName(0, 1, None)))


def test_ensure_anchor_version_noop_when_running_is_older(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.running_version = catalog.parse_bos_version("2025-06-15-0-acde0123-25.06")
    cycle.image_version = catalog.parse_bos_version("2025-07-01-0-0badc0de-25.07")
    backend = _Exec(_routes({}))
    catalog.ensure_anchor_version(Device("h", backend=backend), cycle)
    assert not any("printf" in argv[-1] for argv in backend.runs)
    assert cycle.running_version.canonical == "2025-06-15-0-acde0123-25.06"


@pytest.mark.parametrize("running", ["2025-06-20-0-acde0123-25.07", "2025-09-01-0-acde0123-25.09"])
def test_ensure_anchor_version_rewrites_equal_or_newer_running(
    tmp_path: Path, running: str
) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.running_version = catalog.parse_bos_version(running)
    cycle.image_version = catalog.parse_bos_version("2025-07-01-0-0badc0de-25.07")
    backend = _Exec(_routes({}))
    catalog.ensure_anchor_version(Device("h", backend=backend), cycle)
    anchored = running.replace(running.rsplit("-", 1)[-1], "25.06")
    push = next(argv[-1] for argv in backend.runs if "printf" in argv[-1])
    assert push == f"printf '%s\\n' {anchored} > /etc/bos_version"
    assert cycle.running_version.canonical == anchored


def test_snapshot_bos_version_records_contents(tmp_path: Path) -> None:
    content = b"2025-06-20-0-acde0123-25.07\n"
    backend = _Exec(
        _routes(
            {
                "test -f /etc/bos_version": "present",
                "base64 < /etc/bos_version": base64.b64encode(content).decode(),
                "wc -c < /etc/bos_version": str(len(content)),
                "sha256sum /etc/bos_version": hashlib.sha256(content).hexdigest(),
            }
        )
    )
    cycle = _firmware_cycle(tmp_path)
    catalog.snapshot_bos_version(Device("h", backend=backend), cycle)
    assert cycle.bos_version_snapshot is not None
    assert cycle.bos_version_snapshot.local is not None
    assert cycle.bos_version_snapshot.local.read_bytes() == content


def test_preflight_versions_rejects_malformed_device_version(tmp_path: Path) -> None:
    with pytest.raises(Abort, match="garbage"):
        catalog.preflight_versions(
            Device("h", backend=_Exec(_routes({"cat /etc/bos_version": "garbage"}))),
            _image(tmp_path, version="2025-07-01-0-0badc0de-25.07"),
            _firmware_cycle(tmp_path),
        )


def test_snapshot_remote_file_records_absence_and_restores_by_removal(tmp_path: Path) -> None:
    backend = _Exec(_routes({}))
    dev = Device("h", backend=backend)
    snap = catalog.snapshot_remote_file(dev, "/missing", tmp_path / "missing")
    assert snap.local is None
    catalog.restore_remote_file(dev, snap)
    assert any(argv[-1] == "rm -f /missing" for argv in backend.runs)


def test_snapshot_remote_file_is_byte_exact_and_restores_by_push(tmp_path: Path) -> None:
    content = b"preserve trailing newline\n"
    digest = hashlib.sha256(content).hexdigest()
    backend = _Exec(
        _routes(
            {
                "test -f /config": "present",
                "base64 < /config": base64.b64encode(content).decode(),
                "wc -c < /config": str(len(content)),
                "sha256sum /config": digest,
            }
        )
    )
    dev = Device("h", backend=backend)
    snap = catalog.snapshot_remote_file(dev, "/config", tmp_path / "config.snapshot")
    assert snap.local is not None
    assert snap.local.read_bytes() == content
    catalog.restore_remote_file(dev, snap)
    assert backend.streams[-1][1] == content


def test_restore_servers_config_removes_only_new_quarantine_siblings(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.servers_snapshot = catalog.FileSnapshot(catalog.SERVERS_JSON, None)
    cycle.bcp_before = frozenset({"servers.json.bcp"})
    backend = _Exec(_routes({"servers.json.bcp*": "servers.json.bcp\nservers.json.bcp.1"}))
    catalog.restore_servers_config(Device("h", backend=backend), cycle)
    commands = [argv[-1] for argv in backend.runs]
    assert "rm -f /etc/nix-upgrade/servers.json.bcp.1" in commands
    assert "rm -f /etc/nix-upgrade/servers.json.bcp" not in commands


def _dir_snapshot_backend(payload: bytes, *, reported: bytes | None = None) -> _Exec:
    checked = payload if reported is None else reported
    return _Exec(
        _routes(
            {
                "test -d /etc/opkg/keys": "present",
                "base64 < /tmp/bmc-e2e-opkg-keys": base64.b64encode(payload).decode(),
                "wc -c < /tmp/bmc-e2e-opkg-keys": str(len(checked)),
                "sha256sum /tmp/bmc-e2e-opkg-keys": hashlib.sha256(checked).hexdigest(),
            }
        )
    )


def test_snapshot_remote_dir_is_byte_exact_and_removes_temp(tmp_path: Path) -> None:
    payload = b"tar\x00bytes\n"
    backend = _dir_snapshot_backend(payload)
    snap = catalog.snapshot_remote_dir(
        Device("h", backend=backend), "/etc/opkg/keys", tmp_path / "keys.tar"
    )
    assert snap.archive is not None
    assert snap.archive.read_bytes() == payload
    assert any(argv[-1].startswith("rm -f /tmp/bmc-e2e-opkg-keys") for argv in backend.runs)


def test_snapshot_remote_dir_truncation_aborts_and_removes_temp(tmp_path: Path) -> None:
    backend = _dir_snapshot_backend(b"short", reported=b"expected complete tar")
    with pytest.raises(Abort, match="snapshot"):
        catalog.snapshot_remote_dir(
            Device("h", backend=backend), "/etc/opkg/keys", tmp_path / "keys.tar"
        )
    assert any(argv[-1].startswith("rm -f /tmp/bmc-e2e-opkg-keys") for argv in backend.runs)


def test_restore_remote_dir_deletes_then_recreates_or_leaves_absent(tmp_path: Path) -> None:
    archive = tmp_path / "keys.tar"
    archive.write_bytes(b"archive")
    backend = _Exec(_routes({}))
    dev = Device("h", backend=backend)
    catalog.restore_remote_dir(dev, catalog.DirSnapshot("/etc/opkg/keys", archive))
    commands = [argv[-1] for argv in backend.runs]
    delete_index = commands.index("rm -rf /etc/opkg/keys")
    extract_index = next(
        i for i, cmd in enumerate(commands) if cmd.startswith("tar -C /etc/opkg -xf")
    )
    assert delete_index < extract_index
    assert backend.streams[-1][1] == b"archive"

    absent_backend = _Exec(_routes({}))
    catalog.restore_remote_dir(
        Device("h", backend=absent_backend), catalog.DirSnapshot("/etc/opkg/keys", None)
    )
    assert [argv[-1] for argv in absent_backend.runs] == ["rm -rf /etc/opkg/keys"]


def test_require_nix_era_accepts_complete_payload_and_rejects_missing(tmp_path: Path) -> None:
    catalog.require_nix_era(
        _image(tmp_path, extra=("rootfs.img", "bmc-nix-cli", "servers.json.default"))
    )
    with pytest.raises(Abort, match=r"servers\.json\.default"):
        catalog.require_nix_era(
            _image(tmp_path, extra=("rootfs.img", "bmc-nix-cli"), name="missing.tar")
        )


@pytest.mark.parametrize(
    ("routes", "hint"),
    [
        ({}, "base64"),
        ({"command -v base64": "ok"}, "bmc-nix-cli"),
        (
            {
                "command -v base64": "ok",
                f"test -x {catalog._NIX_CLI}": "ok",
                f"test -f {catalog.SERVERS_JSON}": "present",
                f"cat {catalog.SERVERS_JSON}": "not-json",
            },
            "JSON",
        ),
    ],
)
def test_preflight_device_rejects_missing_or_invalid_prerequisites(
    routes: dict[str, str], hint: str
) -> None:
    with pytest.raises(Abort, match=hint):
        catalog.preflight_device(Device("h", backend=_Exec(_routes(routes))))


def _scan_routes(scans: list[str], routes: dict[str, str] | None = None) -> _Respond:
    remaining = iter(scans)
    fallback = _routes(routes or {})

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        if argv and argv[0] == "ssh" and "for exe in /proc/[0-9]*/exe" in argv[-1]:
            return _cp(argv, next(remaining))
        return fallback(argv)

    return respond


_ENV_LINE = '\tprocd_set_param env "PATH=/usr/sbin:/bin" "XDG_RUNTIME_DIR=/tmp/runtime"'
_INIT_SCRIPT_BODY = "\n".join(
    (
        "#!/bin/sh /etc/rc.common",
        "start_service() {",
        "\tprocd_set_param command /bin/ash -c 'exec \"/nix/store/a/bin/bmc-openwrt\"'",
        _ENV_LINE,
        "\tprocd_set_param respawn 3600 5 0",
        "}",
    )
)


def _pointed_cycle(tmp_path: Path, script: str) -> catalog.FirmwareCycle:
    cycle = _firmware_cycle(tmp_path)
    cycle.host = "192.0.2.5"
    local = tmp_path / "bmc-compositor.init"
    local.write_text(script)
    cycle.init_script_snapshot = catalog.FileSnapshot("/etc/init.d/bmc-compositor", local)
    return cycle


def test_snapshot_service_script_records_contents_and_rejects_missing(tmp_path: Path) -> None:
    content = _INIT_SCRIPT_BODY.encode()
    backend = _Exec(
        _routes(
            {
                "test -f /etc/init.d/bmc-compositor": "present",
                "base64 < /etc/init.d/bmc-compositor": base64.b64encode(content).decode(),
                "wc -c < /etc/init.d/bmc-compositor": str(len(content)),
                "sha256sum /etc/init.d/bmc-compositor": hashlib.sha256(content).hexdigest(),
            }
        )
    )
    cycle = _firmware_cycle(tmp_path)
    catalog.snapshot_service_script(Device("h", backend=backend), cycle)
    assert cycle.init_script_snapshot is not None
    assert cycle.init_script_snapshot.local is not None
    assert cycle.init_script_snapshot.local.read_bytes() == content

    with pytest.raises(Abort, match="missing"):
        catalog.snapshot_service_script(
            Device("h", backend=_Exec(_routes({}))), _firmware_cycle(tmp_path)
        )


def test_scan_bmc_pids_matches_every_store_generation() -> None:
    sweep = "\n".join(
        (
            "101\t/nix/store/old-bmc/bin/bmc-openwrt",
            "202\t/nix/store/new-bmc/bin/bmc-openwrt",
            "303\t/usr/bin/grpcurl",
        )
    )
    dev = Device("h", backend=_Exec(_scan_routes([sweep])))
    assert catalog.scan_bmc_pids(dev) == [101, 202]


def test_quiesce_kills_stale_pid_after_service_stop(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(catalog, "_BMC_KILL_WAIT", 0.0)
    stale = "77\t/nix/store/stale/bin/bmc-openwrt"
    backend = _Exec(_scan_routes([stale, stale, "", ""]))
    catalog.quiesce_bmc(Device("h", backend=backend))
    commands = [argv[-1] for argv in backend.runs]
    assert commands[0] == "service bmc-compositor stop"
    assert "kill -TERM 77" in commands
    assert "kill -KILL 77" in commands


def test_quiesce_raises_when_pid_survives_kill(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(catalog, "_BMC_KILL_WAIT", 0.0)
    stale = "77\t/nix/store/stale/bin/bmc-openwrt"
    with pytest.raises(Abort, match="still running"):
        catalog.quiesce_bmc(Device("h", backend=_Exec(_scan_routes([stale, stale, stale]))))


def test_point_bmc_at_index_injects_env_and_verifies_environ(tmp_path: Path) -> None:
    cycle = _pointed_cycle(tmp_path, _INIT_SCRIPT_BODY)
    backend = _Exec(
        _scan_routes(
            ["222\t/nix/store/a/bin/bmc-openwrt"],
            {
                "wc -c < /var/log/bmc/bmc.log": "12",
                "/proc/222/environ": "BMC_INDEX_URL=http://192.0.2.5:8082",
            },
        )
    )
    catalog.point_bmc_at_index(Device("h", backend=backend), cycle)
    push = next(argv[-1] for argv in backend.runs if argv[-1].startswith("printf"))
    pushed = shlex.split(push)[2]
    assert 'procd_set_param env "BMC_INDEX_URL=http://192.0.2.5:8082" "PATH=' in pushed
    assert pushed.replace('"BMC_INDEX_URL=http://192.0.2.5:8082" ', "", 1) == _INIT_SCRIPT_BODY
    assert push.endswith("> /etc/init.d/bmc-compositor")
    assert any(argv[-1] == "service bmc-compositor restart" for argv in backend.runs)
    assert cycle.bmc_log_offset == 12


def test_point_bmc_at_index_aborts_when_override_already_present(tmp_path: Path) -> None:
    script = _INIT_SCRIPT_BODY.replace('"PATH=', '"BMC_INDEX_URL=http://x" "PATH=')
    cycle = _pointed_cycle(tmp_path, script)
    with pytest.raises(Abort, match="restore the device first"):
        catalog.point_bmc_at_index(Device("h", backend=_Exec(_routes({}))), cycle)


@pytest.mark.parametrize(
    "script",
    [
        _INIT_SCRIPT_BODY.replace(f"{_ENV_LINE}\n", ""),
        _INIT_SCRIPT_BODY.replace(_ENV_LINE, f"{_ENV_LINE}\n{_ENV_LINE}"),
    ],
)
def test_point_bmc_at_index_aborts_without_single_env_line(tmp_path: Path, script: str) -> None:
    cycle = _pointed_cycle(tmp_path, script)
    with pytest.raises(Abort, match="exactly one"):
        catalog.point_bmc_at_index(Device("h", backend=_Exec(_routes({}))), cycle)


def test_point_bmc_at_index_aborts_when_environ_never_matches(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(catalog, "_BMC_RESTART_TIMEOUT", 0.0)
    cycle = _pointed_cycle(tmp_path, _INIT_SCRIPT_BODY)
    backend = _Exec(
        _scan_routes(
            ["222\t/nix/store/a/bin/bmc-openwrt"],
            {"wc -c < /var/log/bmc/bmc.log": "12", "/proc/222/environ": ""},
        )
    )
    with pytest.raises(Abort, match="did not come up"):
        catalog.point_bmc_at_index(Device("h", backend=backend), cycle)


def test_strip_index_override_removes_token_and_flags_foreign(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.host = "192.0.2.5"
    injected = _INIT_SCRIPT_BODY.replace(
        "procd_set_param env ",
        'procd_set_param env "BMC_INDEX_URL=http://192.0.2.5:8082" ',
        1,
    )
    backend = _Exec(_routes({"cat /etc/init.d/bmc-compositor": injected}))
    assert catalog.strip_index_override(Device("h", backend=backend), cycle) is True
    push = next(argv[-1] for argv in backend.runs if argv[-1].startswith("printf"))
    assert shlex.split(push)[2] == _INIT_SCRIPT_BODY

    clean = _Exec(_routes({"cat /etc/init.d/bmc-compositor": _INIT_SCRIPT_BODY}))
    assert catalog.strip_index_override(Device("h", backend=clean), cycle) is False
    assert not any(argv[-1].startswith("printf") for argv in clean.runs)

    foreign = _INIT_SCRIPT_BODY.replace('"PATH=', '"BMC_INDEX_URL=http://other" "PATH=')
    with pytest.raises(Abort, match="unexpected BMC_INDEX_URL"):
        catalog.strip_index_override(
            Device("h", backend=_Exec(_routes({"cat /etc/init.d/bmc-compositor": foreign}))),
            cycle,
        )


def test_kill_bmc_pids_stops_after_term_or_escalates(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(catalog, "_BMC_KILL_WAIT", 0.0)
    stopped = _Exec(_scan_routes([""]))
    catalog.kill_bmc_pids(Device("h", backend=stopped), [10])
    assert [argv[-1] for argv in stopped.runs if "kill -" in argv[-1]] == ["kill -TERM 10"]

    stale = "10\t/nix/store/a/bin/bmc-openwrt"
    escalated = _Exec(_scan_routes([stale, ""]))
    catalog.kill_bmc_pids(Device("h", backend=escalated), [10])
    assert [argv[-1] for argv in escalated.runs if "kill -" in argv[-1]] == [
        "kill -TERM 10",
        "kill -KILL 10",
    ]


def test_await_bmc_ready_reports_log_tail_on_timeout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(catalog, "_BMC_READY_TIMEOUT", 0.0)
    cycle = _firmware_cycle(tmp_path)
    cycle.bmc_log_offset = 7
    backend = _Exec(_routes({"tail -c +8 /var/log/bmc/bmc.log": "new diagnostic"}))
    with pytest.raises(Abort, match="new diagnostic"):
        catalog.await_bmc_ready(Device("h", backend=backend), cycle)


class _IndexState:
    def __init__(self, fetched: bool = True) -> None:
        self.fetched = fetched

    def completed(self, path: str) -> bool:
        assert path == "/index.v1.json"
        return self.fetched


def _checked_firmware_cycle(tmp_path: Path) -> tuple[Image, catalog.FirmwareCycle]:
    image = _image(tmp_path, version="2025-07-01-0-0badc0de-25.07")
    cycle = _firmware_cycle(tmp_path)
    cycle.running_version = catalog.parse_bos_version("2025-06-15-0-acde0123-25.06")
    cycle.image_version = catalog.parse_bos_version(image.version)
    cycle.cookie = "session_id=test"
    return image, cycle


def _firmware_offer(image: Image, cycle: catalog.FirmwareCycle) -> dict[str, object]:
    assert cycle.image_version is not None
    return {
        "upgradeId": "offer-1",
        "firmware": {
            "version": cycle.image_version.canonical,
            "hash": image.sha256.upper(),
            "fileSizeBytes": str(image.size),
        },
        "disruption": "UPGRADE_DISRUPTION_REBOOT",
    }


@pytest.mark.parametrize(
    "response",
    [
        {"enabled": False},
        # grpcurl omits proto3 default-valued fields, so a disabled flag
        # arrives as an empty object.
        {},
    ],
)
def test_require_auto_upgrade_disabled_uses_stock_session(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, response: dict[str, object]
) -> None:
    _, cycle = _checked_firmware_cycle(tmp_path)
    calls: list[tuple[str, str | None]] = []

    def grpcurl(_dev: Device, method: str, **kwargs: object) -> dict[str, object]:
        cookie = kwargs.get("cookie")
        calls.append((method, cookie if isinstance(cookie, str) else None))
        return response

    monkeypatch.setattr(catalog, "_grpcurl", grpcurl)
    catalog.require_auto_upgrade_disabled(Device("h", backend=_Exec(_routes({}))), cycle)
    assert calls == [("UpgradeService/GetAutoUpgrade", "session_id=test")]


def test_require_auto_upgrade_disabled_rejects_enabled(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _, cycle = _checked_firmware_cycle(tmp_path)
    monkeypatch.setattr(catalog, "_grpcurl", lambda *_args, **_kwargs: {"enabled": True})
    with pytest.raises(Abort, match="SetAutoUpgrade"):
        catalog.require_auto_upgrade_disabled(Device("h", backend=_Exec(_routes({}))), cycle)


def test_check_for_firmware_upgrade_accepts_canonical_offer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    image, cycle = _checked_firmware_cycle(tmp_path)
    monkeypatch.setattr(
        catalog, "_grpcurl", lambda *_args, **_kwargs: _firmware_offer(image, cycle)
    )
    catalog.check_for_firmware_upgrade(
        Device("h", backend=_Exec(_routes({}))), image, cycle, _IndexState()
    )
    assert cycle.upgrade_id == "offer-1"


@pytest.mark.parametrize(
    ("change", "hint"),
    [
        (lambda response: response.pop("firmware"), "no firmware"),
        (lambda response: response["firmware"].update(version="wrong"), "version"),
        (lambda response: response["firmware"].update(hash="0" * 64), "hash"),
        (lambda response: response["firmware"].update(fileSizeBytes="1"), "size"),
        (lambda response: response.update(disruption="UPGRADE_DISRUPTION_APP_RESTART"), "REBOOT"),
        (lambda response: response.pop("upgradeId"), "upgrade id"),
    ],
)
def test_check_for_firmware_upgrade_rejects_invalid_offer(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    change: Callable[[dict[str, object]], object],
    hint: str,
) -> None:
    image, cycle = _checked_firmware_cycle(tmp_path)
    response = _firmware_offer(image, cycle)
    change(response)
    monkeypatch.setattr(catalog, "_grpcurl", lambda *_args, **_kwargs: response)
    with pytest.raises(Abort, match=hint):
        catalog.check_for_firmware_upgrade(
            Device("h", backend=_Exec(_routes({}))), image, cycle, _IndexState()
        )


def test_check_for_firmware_upgrade_missing_offer_names_versions_and_response(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    image, cycle = _checked_firmware_cycle(tmp_path)
    monkeypatch.setattr(catalog, "_grpcurl", lambda *_args, **_kwargs: {"disruption": "x"})
    assert cycle.running_version is not None
    assert cycle.image_version is not None
    with pytest.raises(Abort) as error:
        catalog.check_for_firmware_upgrade(
            Device("h", backend=_Exec(_routes({}))), image, cycle, _IndexState()
        )
    for expected in (
        str(cycle.running_version.version),
        str(cycle.image_version.version),
        "disruption",
    ):
        assert expected in error.value.hint


def test_check_for_firmware_upgrade_requires_completed_index_fetch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    image, cycle = _checked_firmware_cycle(tmp_path)
    monkeypatch.setattr(
        catalog, "_grpcurl", lambda *_args, **_kwargs: _firmware_offer(image, cycle)
    )
    with pytest.raises(Abort, match=r"index\.v1\.json"):
        catalog.check_for_firmware_upgrade(
            Device("h", backend=_Exec(_routes({}))), image, cycle, _IndexState(False)
        )


class _StreamProcess:
    def __init__(self, stdout: str, stderr: str = "", returncode: int = 0) -> None:
        self.stdout = io.StringIO(stdout)
        self.stderr = io.StringIO(stderr)
        self.returncode = returncode

    def wait(self) -> int:
        return self.returncode


def test_run_firmware_stream_sets_flag_before_spawn_and_preserves_partial_json(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _, cycle = _checked_firmware_cycle(tmp_path)
    cycle.upgrade_id = "offer-1"

    def popen(argv: list[str], **kwargs: object) -> _StreamProcess:
        assert cycle.started_upgrade is True
        assert argv[argv.index("-d") + 1] == json.dumps({"upgradeId": "offer-1"})
        assert argv[argv.index("-max-time") + 1] == "600"
        assert "-format-error" in argv
        assert kwargs["text"] is True
        return _StreamProcess(
            '{"firmwarePhase":"FIRMWARE_UPGRADE_PHASE_DOWNLOADING"}\n{"download":'
        )

    monkeypatch.setattr(catalog.subprocess, "Popen", popen)
    result = catalog.run_firmware_stream(Device("h", backend=_Exec(_routes({}))), cycle)
    assert result.events == [{"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_DOWNLOADING"}]
    assert '{"download":' in result.stderr


def test_run_firmware_stream_resets_flag_when_spawn_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _, cycle = _checked_firmware_cycle(tmp_path)
    cycle.upgrade_id = "offer-1"

    def popen(*_args: object, **_kwargs: object) -> None:
        assert cycle.started_upgrade is True
        raise OSError("missing grpcurl")

    monkeypatch.setattr(catalog.subprocess, "Popen", popen)
    with pytest.raises(OSError, match="missing grpcurl"):
        catalog.run_firmware_stream(Device("h", backend=_Exec(_routes({}))), cycle)
    assert cycle.started_upgrade is False


def test_run_firmware_stream_decodes_formatted_error_status(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _, cycle = _checked_firmware_cycle(tmp_path)
    cycle.upgrade_id = "offer-1"
    monkeypatch.setattr(
        catalog.subprocess,
        "Popen",
        lambda *_args, **_kwargs: _StreamProcess(
            "", '{"code":13,"message":"failed verification"}', 1
        ),
    )
    result = catalog.run_firmware_stream(Device("h", backend=_Exec(_routes({}))), cycle)
    assert result.status_code == "Internal"
    assert result.status_message == "failed verification"


def _stream_result(
    events: list[dict[str, object]], *, exit_code: int = 1, status: str | None = None
) -> catalog.StreamResult:
    return catalog.StreamResult(events, exit_code, status, None, "")


_DOWNLOADED: list[dict[str, object]] = [
    {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_DOWNLOADING"},
    {"download": {"downloadedBytes": "1", "totalBytes": "2"}},
    {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_VERIFYING"},
]


@pytest.mark.parametrize(
    ("result", "outcome"),
    [
        (
            _stream_result(
                [*_DOWNLOADED, {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_APPLYING"}], exit_code=0
            ),
            catalog.StreamOutcome.PROVISIONAL_SUCCESS,
        ),
        (
            _stream_result(_DOWNLOADED, status="Unavailable"),
            catalog.StreamOutcome.PROVISIONAL_SUCCESS,
        ),
        (_stream_result(_DOWNLOADED), catalog.StreamOutcome.PROVISIONAL_SUCCESS),
        (_stream_result([], status="Unavailable"), catalog.StreamOutcome.REJECTED),
        (
            _stream_result(
                [{"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_DOWNLOADING"}], status="Internal"
            ),
            catalog.StreamOutcome.TERMINAL_FAILURE,
        ),
        (
            _stream_result(_DOWNLOADED, status="FailedPrecondition"),
            catalog.StreamOutcome.TERMINAL_FAILURE,
        ),
        (_stream_result([{"finished": {}}], exit_code=0), catalog.StreamOutcome.TERMINAL_FAILURE),
        (_stream_result([], status="DeadlineExceeded"), catalog.StreamOutcome.POSSIBLY_ACCEPTED),
        (
            _stream_result(_DOWNLOADED[:1], status="Cancelled"),
            catalog.StreamOutcome.POSSIBLY_ACCEPTED,
        ),
        (_stream_result([], status=None), catalog.StreamOutcome.POSSIBLY_ACCEPTED),
        (
            _stream_result([{"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_APPLYING"}], exit_code=0),
            catalog.StreamOutcome.POSSIBLY_ACCEPTED,
        ),
        (
            _stream_result(_DOWNLOADED[:2], status="Unavailable"),
            catalog.StreamOutcome.POSSIBLY_ACCEPTED,
        ),
        (
            _stream_result(
                [
                    {"download": {"downloadedBytes": "1"}},
                    {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_VERIFYING"},
                ],
                status="Unavailable",
            ),
            catalog.StreamOutcome.POSSIBLY_ACCEPTED,
        ),
        (
            _stream_result(
                [
                    {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_DOWNLOADING"},
                    {"firmwarePhase": "FIRMWARE_UPGRADE_PHASE_VERIFYING"},
                ],
                status="Unavailable",
            ),
            catalog.StreamOutcome.POSSIBLY_ACCEPTED,
        ),
        (
            _stream_result([*_DOWNLOADED, {"finished": {}}], exit_code=0),
            catalog.StreamOutcome.TERMINAL_FAILURE,
        ),
    ],
)
def test_classify_firmware_stream(
    result: catalog.StreamResult, outcome: catalog.StreamOutcome
) -> None:
    assert catalog.classify_stream(result) is outcome


def _boot_id_routes(responses: list[str | OSError | None]) -> _Respond:
    remaining = iter(responses)

    def respond(argv: list[str]) -> "subprocess.CompletedProcess[str]":
        if argv and argv[0] == "ssh" and argv[-1] == "cat /tmp/wifi_mac":
            return _cp(argv, "88:a6:ef:d1:17:6e\n")
        if argv and argv[0] == "ssh" and argv[-1] == "cat /proc/sys/kernel/random/boot_id":
            response = next(remaining)
            if response is None:
                raise subprocess.CalledProcessError(255, argv)
            if isinstance(response, OSError):
                raise response
            return _cp(argv, response)
        return _cp(argv)

    return respond


def _poll_time() -> tuple[Callable[[float], None], Callable[[], float]]:
    now = 0.0

    def sleep(seconds: float) -> None:
        nonlocal now
        now += seconds

    return sleep, lambda: now


def test_snapshot_boot_id_records_the_current_boot(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    catalog.snapshot_boot_id(
        Device("h", backend=_Exec(_routes({"cat /proc/sys/kernel/random/boot_id": "boot-a"}))),
        cycle,
    )
    assert cycle.boot_id_before == "boot-a"
    assert catalog.BOOT_POLL_TIMEOUT == 180.0


def test_snapshot_and_verify_device_identity(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    snapshot_dev = Device(
        "h",
        backend=_Exec(_routes({"cat /tmp/wifi_mac": " \t88:A6:EF:D1:17:6E\0\n"})),
    )
    verify_dev = Device(
        "h",
        backend=_Exec(_routes({"cat /tmp/wifi_mac": "88:a6:ef:d1:17:6e\n"})),
    )
    catalog.snapshot_device_identity(snapshot_dev, cycle)
    assert cycle.device_identity == "88:a6:ef:d1:17:6e"
    assert catalog.verify_device_identity(verify_dev, cycle) == "88:a6:ef:d1:17:6e"


def test_verify_device_identity_rejects_a_different_device(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.device_identity = "88:a6:ef:d1:17:6e"
    dev = Device(
        "h",
        backend=_Exec(_routes({"cat /tmp/wifi_mac": "aa:bb:cc:dd:ee:ff\n"})),
    )
    with pytest.raises(Abort, match="device identity changed"):
        catalog.verify_device_identity(dev, cycle)


def test_poll_boot_id_change_tolerates_ssh_failure_until_new_boot(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.boot_id_before = "boot-a"
    sleep, clock = _poll_time()
    backend = _Exec(_boot_id_routes(["boot-a", None, "boot-a", "boot-b"]))
    assert catalog.poll_boot_id_change(
        Device("h", backend=backend), cycle, timeout=4.0, sleep=sleep, clock=clock
    )


def test_poll_boot_id_change_tolerates_os_error(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.boot_id_before = "boot-a"
    sleep, clock = _poll_time()
    backend = _Exec(_boot_id_routes([OSError("ssh missing"), "boot-b"]))
    assert catalog.poll_boot_id_change(
        Device("h", backend=backend), cycle, timeout=2.0, sleep=sleep, clock=clock
    )


def test_poll_boot_id_change_times_out_when_boot_never_changes(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.boot_id_before = "boot-a"
    sleep, clock = _poll_time()
    assert not catalog.poll_boot_id_change(
        Device(
            "h",
            backend=_Exec(
                _routes(
                    {
                        "cat /tmp/wifi_mac": "88:a6:ef:d1:17:6e\n",
                        "cat /proc/sys/kernel/random/boot_id": "boot-a",
                    }
                )
            ),
        ),
        cycle,
        timeout=3.0,
        sleep=sleep,
        clock=clock,
    )


def test_poll_boot_id_change_rejects_plain_reachability(tmp_path: Path) -> None:
    cycle = _firmware_cycle(tmp_path)
    cycle.boot_id_before = "boot-a"
    sleep, clock = _poll_time()
    backend = _Exec(
        _routes(
            {
                "cat /tmp/wifi_mac": "88:a6:ef:d1:17:6e\n",
                "cat /proc/sys/kernel/random/boot_id": "boot-a",
            }
        )
    )
    assert not catalog.poll_boot_id_change(
        Device("h", backend=backend), cycle, timeout=1.0, sleep=sleep, clock=clock
    )
    assert backend.runs


def test_read_flashed_version_parses_raw_version_canonically() -> None:
    dev = Device(
        "h",
        backend=_Exec(_routes({"cat /etc/bos_version": "2025-07-01-007-0BADC0DE-25.7"})),
    )
    assert catalog.read_flashed_version(dev).canonical == "2025-07-01-7-0badc0de-25.07"


def test_read_flashed_version_propagates_parse_error() -> None:
    dev = Device("h", backend=_Exec(_routes({"cat /etc/bos_version": "garbage"})))
    with pytest.raises(ValueError):
        catalog.read_flashed_version(dev)


def test_verify_stock_service_accepts_running_and_rejects_stopped() -> None:
    running = _routes({"service bmc-compositor status": "running"})
    catalog.verify_stock_service(Device("h", backend=_Exec(running)))

    with pytest.raises(Abort, match="bmc-compositor"):
        catalog.verify_stock_service(Device("h", backend=_Exec(_routes({}))))


class _OrderedExec(_Exec):
    def stream(self, argv: list[str], chunks: Iterable[bytes]) -> None:
        self.runs.append(argv)
        super().stream(argv, chunks)


def _restorable_cycle(tmp_path: Path) -> catalog.FirmwareCycle:
    cycle = _firmware_cycle(tmp_path)
    servers = tmp_path / "servers.json"
    nix_conf = tmp_path / "nix.conf"
    opkg_keys = tmp_path / "opkg-keys.tar"
    servers.write_text("servers")
    nix_conf.write_text("nix-conf")
    opkg_keys.write_text("keys")
    cycle.servers_snapshot = catalog.FileSnapshot(catalog.SERVERS_JSON, servers)
    cycle.nix_conf_snapshot = catalog.FileSnapshot(catalog._NIX_CONF, nix_conf)
    cycle.opkg_keys_snapshot = catalog.DirSnapshot("/etc/opkg/keys", opkg_keys)
    return cycle


def test_restore_after_success_quiesces_before_restores_and_start(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(catalog, "_BMC_KILL_WAIT", 0.0)
    stale = "77\t/nix/store/stale/bin/bmc-openwrt"
    backend = _OrderedExec(
        _scan_routes(
            [stale, stale, "", ""],
            {"service bmc-compositor status": "running"},
        )
    )
    cycle = _restorable_cycle(tmp_path)
    catalog.restore_after_success(Device("h", backend=backend), cycle)
    commands = [argv[-1] for argv in backend.runs]
    server_push = commands.index(f"cat > {catalog.SERVERS_JSON}")
    nix_push = commands.index(f"cat > {catalog._NIX_CONF}")
    scan_indices = [
        index for index, command in enumerate(commands) if "for exe in /proc/[0-9]*/exe" in command
    ]
    assert commands.index("service bmc-compositor stop") < commands.index("kill -TERM 77")
    assert commands.index("kill -KILL 77") < max(scan_indices) < server_push < nix_push
    assert nix_push < commands.index("service bmc-compositor start")
    assert not any("/etc/opkg/keys" in command for command in commands)
    assert cycle.opkg_keys_snapshot is None


def test_restore_after_success_strips_surviving_index_override(tmp_path: Path) -> None:
    # keep.d preserves the init script across sysupgrade, so the injected
    # env var rides the flash and must be stripped on the new system.
    cycle = _restorable_cycle(tmp_path)
    cycle.host = "192.0.2.5"
    injected = _INIT_SCRIPT_BODY.replace(
        "procd_set_param env ",
        'procd_set_param env "BMC_INDEX_URL=http://192.0.2.5:8082" ',
        1,
    )
    backend = _OrderedExec(
        _scan_routes(
            ["", ""],
            {
                "service bmc-compositor status": "running",
                "cat /etc/init.d/bmc-compositor": injected,
            },
        )
    )
    catalog.restore_after_success(Device("h", backend=backend), cycle)
    commands = [argv[-1] for argv in backend.runs]
    push = next(command for command in commands if command.startswith("printf"))
    assert shlex.split(push)[2] == _INIT_SCRIPT_BODY
    assert commands.index(push) < commands.index("service bmc-compositor start")


def test_restore_after_success_aborts_if_stock_service_does_not_start(
    tmp_path: Path,
) -> None:
    backend = _OrderedExec(_scan_routes(["", ""]))
    with pytest.raises(Abort, match="bmc-compositor"):
        catalog.restore_after_success(Device("h", backend=backend), _restorable_cycle(tmp_path))
    commands = [argv[-1] for argv in backend.runs]
    assert commands.index(f"cat > {catalog.SERVERS_JSON}") < commands.index(
        "service bmc-compositor start"
    )
    assert commands.index(f"cat > {catalog._NIX_CONF}") < commands.index(
        "service bmc-compositor start"
    )


def test_sweep_store_ballast_removes_both_the_ballast_and_its_spacer() -> None:
    # The spacer is a separate file, and a run killed between the fill and
    # its removal leaves it behind: sweeping only the ballast would keep
    # eating the margin the next fill is trying to measure.
    backend = _Exec(_routes({}))
    catalog.sweep_store_ballast(Device("h", backend=backend))
    removed = " ".join(argv[-1] for argv in backend.runs)
    assert catalog._STORE_BALLAST in removed
    assert catalog._STORE_BALLAST_SPACER in removed


def test_store_available_kib_aborts_on_a_non_numeric_available_field() -> None:
    malformed = f"filesystem 100 0 unknown 0% {catalog._DATA_MOUNT}"
    backend = _Exec(_routes({f"df -k {catalog._DATA_MOUNT}": malformed}))
    with pytest.raises(Abort, match="unparseable df output"):
        catalog._store_available_kib(Device("h", backend=backend))


def test_fill_store_filesystem_reopens_the_margin_only_after_hitting_zero() -> None:
    """The fill is what makes the fault fire: it must run the store to a
    genuine ENOSPC — root spends ext4's reserve, so `df` reaching 0 is the
    only proof — and only then delete the pre-sized spacer, which is the
    sole way to reopen an exact margin without `fallocate` or `truncate`."""
    df_zero = f"filesystem 100 100 0 100% {catalog._DATA_MOUNT}"
    backend = _Exec(_routes({f"df -k {catalog._DATA_MOUNT}": df_zero}))
    catalog.fill_store_filesystem(Device("h", backend=backend))

    commands = [argv[-1] for argv in backend.runs]
    spacer_written = next(
        i for i, c in enumerate(commands) if "dd" in c and catalog._STORE_BALLAST_SPACER in c
    )
    filled = next(
        i for i, c in enumerate(commands) if "dd" in c and f"of={catalog._STORE_BALLAST} " in c
    )
    spacer_removed = next(
        i for i, c in enumerate(commands) if c.startswith("rm -f") and "spacer" in c
    )
    assert spacer_written < filled < spacer_removed


def test_fill_store_filesystem_aborts_when_the_store_did_not_fill() -> None:
    # A fill that stopped short leaves room for the upgrade, the flash
    # succeeds, and C7 would report a fault that never fired.
    df_free = f"filesystem 100 0 4096 0% {catalog._DATA_MOUNT}"
    backend = _Exec(_routes({f"df -k {catalog._DATA_MOUNT}": df_free}))
    with pytest.raises(Abort, match="still has 4096 KiB free"):
        catalog.fill_store_filesystem(Device("h", backend=backend))

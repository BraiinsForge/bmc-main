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

"""Validate widget reload and wasm-host lifecycle behavior on a real Deck."""

import json
import shlex
import subprocess
import tempfile
import threading
import time
import uuid
from collections import Counter
from collections.abc import Callable, Sequence
from contextlib import chdir
from dataclasses import asdict, dataclass
from enum import StrEnum
from pathlib import Path
from typing import Literal

from bmc_tui import catalog, console
from bmc_tui.device import Device
from bmc_tui.procedures.deploy import Deploy
from bmc_tui.stage import Abort, entrypoint, require

_TMP = Path(tempfile.gettempdir()) / "bmc-widget-host-e2e"
_WIDGET_SOURCE = Path("widgets-wasm/blockheight/src/lib.rs")
_HOST_SOURCE = Path("bmc-wasm-host/src/main.rs")
_BLOCKHEIGHT = "blockheight"
_SAMPLE_INTERVAL_SECONDS = 1
_PROCESS_FIELDS = 5
_ONE_RESTART_PID_COUNT = 2
_TRACKED_STATUS = ("status", "--porcelain", "--untracked-files=no")
_COMPOSITOR_PID_FILE = "/var/run/bmc-compositor.pid"
PROCESS_STARTTIME_FIELD_AFTER_COMM = 20
SNAPSHOT_COMMAND = rf"""
for name in bmc-openwrt bmc-wasm-host bmc-wasm-thin; do
  for pid in $(pidof "$name" 2>/dev/null); do
    start=$(sed 's/.*) //' /proc/$pid/stat | awk '{{print ${PROCESS_STARTTIME_FIELD_AFTER_COMM}}}')
    exe=$(readlink /proc/$pid/exe)
    cmd=$(tr '\000' ' ' < /proc/$pid/cmdline)
    printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$pid" "$start" "$exe" "$cmd"
  done
done
"""


class _Role(StrEnum):
    COMPOSITOR = "bmc-openwrt"
    HOST = "bmc-wasm-host"
    THIN = "bmc-wasm-thin"


class _ProfileGeneration(StrEnum):
    LEGACY = "legacy"
    CURRENT = "current"


@dataclass(frozen=True)
class Process:
    role: _Role
    pid: int
    starttime: int
    executable: str
    cmdline: tuple[str, ...]

    @property
    def identity(self) -> tuple[int, int]:
        return self.pid, self.starttime

    @property
    def wasm(self) -> str | None:
        try:
            index = self.cmdline.index("--wasm")
        except ValueError:
            return None
        return self.cmdline[index + 1] if index + 1 < len(self.cmdline) else None

    @property
    def widget(self) -> str | None:
        wasm = self.wasm
        if wasm is None or "-bmc-widget-" not in wasm:
            return None
        return wasm.split("-bmc-widget-", 1)[1].split("/", 1)[0]


@dataclass(frozen=True)
class Snapshot:
    compositor: Process
    host: Process
    thins: tuple[Process, ...]

    @property
    def blockheight(self) -> tuple[Process, ...]:
        return tuple(thin for thin in self.thins if thin.widget == _BLOCKHEIGHT)

    @property
    def inventory(self) -> Counter[str | None]:
        return Counter(thin.widget for thin in self.thins)


def parse_snapshot(raw: str) -> Snapshot:
    found: dict[_Role, list[Process]] = {role: [] for role in _Role}
    for line in raw.splitlines():
        fields = line.split("\t", 4)
        if len(fields) != _PROCESS_FIELDS:
            raise ValueError(f"malformed process row: {line!r}")
        role_text, pid, starttime, executable, cmdline = fields
        try:
            role = _Role(role_text)
            process = Process(
                role=role,
                pid=int(pid),
                starttime=int(starttime),
                executable=executable,
                cmdline=tuple(shlex.split(cmdline)),
            )
        except (ValueError, TypeError) as error:
            raise ValueError(f"malformed process row: {line!r}") from error
        found[role].append(process)

    if len(found[_Role.COMPOSITOR]) != 1:
        raise ValueError(f"expected one compositor, found {len(found[_Role.COMPOSITOR])}")
    if len(found[_Role.HOST]) != 1:
        raise ValueError(f"expected one wasm host, found {len(found[_Role.HOST])}")
    if not found[_Role.THIN]:
        raise ValueError("expected at least one wasm thin")
    return Snapshot(
        compositor=found[_Role.COMPOSITOR][0],
        host=found[_Role.HOST][0],
        thins=tuple(found[_Role.THIN]),
    )


def _wait_healthy(
    read: Callable[[], str],
    *,
    timeout: float = 120,
    expected_thins: int | None = None,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> Snapshot:
    deadline = clock() + timeout
    failure = "no process snapshot"
    while True:
        try:
            snapshot = parse_snapshot(read())
            if expected_thins is not None and len(snapshot.thins) != expected_thins:
                raise ValueError(
                    f"expected {expected_thins} wasm thins, found {len(snapshot.thins)}"
                )
            return snapshot
        except ValueError as error:
            failure = str(error)
        if clock() >= deadline:
            raise Abort(f"widget host did not become healthy within {timeout:g}s: {failure}")
        sleep(0.2)


class _Transition(StrEnum):
    NONE = "no transition"
    ONE = "one restart"


def _parse_pid_sample(line: str) -> frozenset[int]:
    if line.strip() in {"", "-"}:
        return frozenset()
    try:
        return frozenset(int(pid) for pid in line.split())
    except ValueError as error:
        raise ValueError(f"malformed compositor PID sample: {line!r}") from error


def _classify_trace(old_pid: int, samples: Sequence[frozenset[int]]) -> _Transition:
    observed = set().union(*samples) if samples else set()
    require(old_pid in observed, f"PID trace did not observe starting compositor {old_pid}")
    if observed == {old_pid}:
        return _Transition.NONE
    require(
        len(observed) == _ONE_RESTART_PID_COUNT,
        f"PID trace observed more than one compositor restart: {sorted(observed)}",
    )
    return _Transition.ONE


def _complete_trace(
    samples: Sequence[frozenset[int]], endpoint_pid: int
) -> tuple[frozenset[int], ...]:
    require(bool(samples), "compositor PID sampler produced no samples")
    return (*samples, frozenset({endpoint_pid}))


def _sampler_command(marker: str) -> str:
    return (
        f"rm -f {marker}; "
        f"while [ ! -e {marker} ]; do "
        f"p=$(cat {_COMPOSITOR_PID_FILE} 2>/dev/null || true); "
        'case "$p" in ""|*[!0-9]*) p=;; esac; '
        'exe=$([ -n "$p" ] && readlink "/proc/$p/exe" 2>/dev/null || true); '
        'case "$exe" in */bin/bmc-openwrt) printf \'%s\\n\' "$p";; '
        "*) printf '%s\\n' -;; esac; "
        f"sleep {_SAMPLE_INTERVAL_SECONDS}; done; rm -f {marker}"
    )


class _PidSampler:
    def __init__(self, dev: Device) -> None:
        self._dev = dev
        self._marker = f"/tmp/bmc-widget-host-e2e-sampler-{uuid.uuid4().hex}"
        self._samples: list[frozenset[int]] = []
        self._ready = threading.Event()
        self._error: BaseException | None = None
        self._thread = threading.Thread(target=self._run, name="compositor-pid-sampler")

    @property
    def samples(self) -> tuple[frozenset[int], ...]:
        return tuple(self._samples)

    def _line(self, line: str) -> None:
        self._samples.append(_parse_pid_sample(line))
        self._ready.set()

    def _run(self) -> None:
        try:
            self._dev.run_streamed(_sampler_command(self._marker), on_line=self._line)
        except BaseException as error:
            self._error = error
            self._ready.set()

    def start(self) -> None:
        self._thread.start()
        require(self._ready.wait(10), "compositor PID sampler did not produce a sample")
        if self._error is not None:
            raise self._error

    def stop(self) -> None:
        self._dev.run(f"touch {self._marker}")
        self._thread.join(10)
        require(not self._thread.is_alive(), "compositor PID sampler did not stop")
        if self._error is not None:
            raise self._error


def _is_lfs_pointer(content: bytes) -> bool:
    return content.startswith(b"version https://git-lfs.github.com/spec/v1\n")


def _unresolved_lfs(paths: Sequence[Path], read: Callable[[Path], bytes]) -> list[Path]:
    return [path for path in paths if _is_lfs_pointer(read(path))]


def _require_tracked_clean(status: str) -> None:
    require(not status.strip(), "tracked worktree changes would make recovery ambiguous")


def _profile_generation(read: Callable[[str], str]) -> _ProfileGeneration:
    return (
        _ProfileGeneration.CURRENT
        if catalog.widget_reload_available(read)
        else _ProfileGeneration.LEGACY
    )


def _require_baseline_transition(
    generation: _ProfileGeneration, before: Snapshot, after: Snapshot
) -> None:
    if generation == _ProfileGeneration.LEGACY:
        require(
            after.host.identity != before.host.identity,
            "baseline deploy left the legacy wasm host running",
        )


def _complete_baseline(
    generation: _ProfileGeneration,
    before: Snapshot,
    after: Snapshot,
    passed: Callable[[str], None],
) -> None:
    _require_baseline_transition(generation, before, after)
    passed("baseline")


_Git = Callable[[Path, Sequence[str]], str]
_Append = Callable[[Path, str], None]


def _real_git(cwd: Path, args: Sequence[str]) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def _append(path: Path, content: str) -> None:
    with path.open("a") as output:
        output.write(content)


@dataclass(frozen=True)
class _Variant:
    name: str
    edits: tuple[tuple[Path, str], ...]
    packages: tuple[str, ...]


_VARIANTS = (
    _Variant(
        "widget",
        ((_WIDGET_SOURCE, "\n// widget-host-e2e: widget variant\n"),),
        ("widget-blockheight",),
    ),
    _Variant(
        "host",
        ((_HOST_SOURCE, "\n// widget-host-e2e: host variant\n"),),
        ("core",),
    ),
    _Variant(
        "combined",
        (
            (_WIDGET_SOURCE, "\n// widget-host-e2e: combined widget variant\n"),
            (_HOST_SOURCE, "\n// widget-host-e2e: combined host variant\n"),
        ),
        ("core", "widget-blockheight"),
    ),
)


class _VariantWorktrees:
    def __init__(
        self,
        repo: Path,
        root: Path,
        *,
        git: _Git = _real_git,
        append: _Append = _append,
    ) -> None:
        self._repo = repo
        self._root = root
        self._git = git
        self._append = append
        self._created: list[Path] = []
        self.paths: dict[str, Path] = {}

    def create(self) -> dict[str, Path]:
        self._root.mkdir(parents=True, exist_ok=True)
        try:
            for variant in _VARIANTS:
                path = self._root / variant.name
                require(not path.exists(), f"temporary worktree already exists: {path}")
                self._git(self._repo, ("worktree", "add", "--detach", str(path), "HEAD"))
                self._created.append(path)
                listed = self._git(path, ("lfs", "ls-files", "--name-only"))
                lfs_paths = [path / name for name in listed.splitlines() if name]
                unresolved = _unresolved_lfs(lfs_paths, Path.read_bytes)
                unresolved_names = ", ".join(str(item.relative_to(path)) for item in unresolved)
                require(
                    not unresolved,
                    f"Git LFS pointers are unresolved in {variant.name}: {unresolved_names}",
                )
                for source, comment in variant.edits:
                    self._append(path / source, comment)
                self.paths[variant.name] = path
        except BaseException:
            self.cleanup()
            raise
        return self.paths

    def cleanup(self) -> None:
        for path in reversed(self._created):
            self._git(self._repo, ("worktree", "remove", "--force", str(path)))
        self._created.clear()


def _finish(
    run: Callable[[], None],
    recover: Callable[[], None],
    cleanup: Callable[[], None],
) -> None:
    try:
        run()
    finally:
        try:
            recover()
        finally:
            cleanup()


def _process_payload(process: Process) -> dict[str, object]:
    return asdict(process)


class Evidence:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.root.mkdir(parents=True, exist_ok=True)

    def snapshot(self, name: str, snapshot: Snapshot) -> None:
        payload = {
            "compositor": _process_payload(snapshot.compositor),
            "host": _process_payload(snapshot.host),
            "thins": [_process_payload(thin) for thin in snapshot.thins],
        }
        (self.root / f"{name}-snapshot.json").write_text(
            json.dumps(payload, indent=2, default=str) + "\n"
        )

    def trace(self, name: str, samples: Sequence[frozenset[int]]) -> None:
        lines = [" ".join(str(pid) for pid in sorted(sample)) or "-" for sample in samples]
        (self.root / f"{name}-pid-trace.txt").write_text("\n".join(lines) + "\n")

    def text(self, name: str, content: str) -> None:
        (self.root / name).write_text(content)

    def passed(self, scenario: str) -> None:
        with (self.root / "status.txt").open("a") as output:
            output.write(f"{scenario}: PASS\n")


@dataclass
class WidgetHostE2e:
    device: str  # IP or host of the target Deck
    profile: Literal["release", "debug"] = "release"  # build profile for every variant
    max_jobs: int | None = None  # nix --max-jobs for builds; None uses nix's own config

    def _deploy(self, cwd: Path, packages: Sequence[str]) -> None:
        with chdir(cwd):
            Deploy(
                device=self.device,
                packages=list(packages),
                profile=self.profile,
                max_jobs=self.max_jobs,
            ).run()

    @staticmethod
    def _snapshot(dev: Device, expected_thins: int | None = None) -> Snapshot:
        return _wait_healthy(lambda: dev.read(SNAPSHOT_COMMAND), expected_thins=expected_thins)

    def _deploy_traced(
        self, dev: Device, cwd: Path, packages: Sequence[str]
    ) -> tuple[Snapshot, tuple[frozenset[int], ...]]:
        before = self._snapshot(dev)
        sampler = _PidSampler(dev)
        sampler.start()
        try:
            self._deploy(cwd, packages)
            after = self._snapshot(dev, len(before.thins))
        finally:
            sampler.stop()
        return after, _complete_trace(sampler.samples, after.compositor.pid)

    @staticmethod
    def _require_inventory(reference: Snapshot, current: Snapshot) -> None:
        require(
            current.inventory == reference.inventory,
            f"widget inventory changed: {reference.inventory} -> {current.inventory}",
        )

    def _baseline(
        self, dev: Device, repo: Path, evidence: Evidence, pre_feature: Snapshot
    ) -> Snapshot:
        self._deploy(repo, ())
        baseline = self._snapshot(dev, len(pre_feature.thins))
        require(bool(baseline.blockheight), "baseline has no Blockheight instance")
        evidence.snapshot("baseline", baseline)
        return baseline

    def _widget_only(
        self, dev: Device, path: Path, evidence: Evidence, baseline: Snapshot
    ) -> Snapshot:
        widget, trace = self._deploy_traced(dev, path, _VARIANTS[0].packages)
        require(
            widget.compositor.identity == baseline.compositor.identity,
            "widget-only deploy restarted the compositor",
        )
        require(
            widget.host.identity == baseline.host.identity,
            "widget-only deploy replaced host",
        )
        baseline_other = {thin.identity for thin in baseline.thins if thin.widget != _BLOCKHEIGHT}
        widget_other = {thin.identity for thin in widget.thins if thin.widget != _BLOCKHEIGHT}
        require(widget_other == baseline_other, "widget-only deploy replaced a non-target thin")
        replaced = {thin.identity for thin in widget.blockheight}.isdisjoint(
            thin.identity for thin in baseline.blockheight
        )
        require(
            len(widget.blockheight) == len(baseline.blockheight) and replaced,
            "widget-only deploy did not replace every Blockheight thin",
        )
        require(
            {thin.wasm for thin in widget.blockheight}
            != {thin.wasm for thin in baseline.blockheight},
            "widget-only deploy did not change the Blockheight store target",
        )
        require(
            _classify_trace(baseline.compositor.pid, trace) == _Transition.NONE,
            "widget-only deploy observed a compositor transition",
        )
        evidence.snapshot("widget-only", widget)
        evidence.trace("widget-only", trace)
        evidence.passed("widget-only")
        return widget

    def _host_only(
        self,
        dev: Device,
        path: Path,
        evidence: Evidence,
        baseline: Snapshot,
        widget: Snapshot,
    ) -> Snapshot:
        host, trace = self._deploy_traced(dev, path, _VARIANTS[1].packages)
        self._require_inventory(baseline, host)
        require(
            _classify_trace(widget.compositor.pid, trace) == _Transition.ONE,
            "host-only deploy did not observe exactly one compositor restart",
        )
        require(
            host.compositor.identity != widget.compositor.identity,
            "host-only compositor held",
        )
        require(
            host.host.identity != widget.host.identity,
            "host-only deploy retained the old host",
        )
        require(
            host.host.starttime >= host.compositor.starttime,
            "host-only wasm host predates its compositor",
        )
        require(
            {thin.wasm for thin in host.blockheight} == {thin.wasm for thin in widget.blockheight},
            "host-only deploy changed the Blockheight store target",
        )
        evidence.snapshot("host-only", host)
        evidence.trace("host-only", trace)
        evidence.passed("host-only")
        return host

    def _combined(
        self,
        dev: Device,
        path: Path,
        widget: Snapshot,
        host: Snapshot,
    ) -> tuple[Snapshot, tuple[frozenset[int], ...]]:
        combined, trace = self._deploy_traced(dev, path, _VARIANTS[2].packages)
        self._require_inventory(host, combined)
        require(
            _classify_trace(host.compositor.pid, trace) == _Transition.ONE,
            "combined deploy did not observe exactly one compositor restart",
        )
        require(
            combined.host.identity != host.host.identity,
            "combined deploy retained old host",
        )
        require(
            combined.host.starttime >= combined.compositor.starttime,
            "combined wasm host predates its compositor",
        )
        require(
            {thin.wasm for thin in combined.blockheight}
            != {thin.wasm for thin in widget.blockheight},
            "combined deploy did not install its Blockheight target",
        )
        time.sleep(2)
        quiet = self._snapshot(dev, len(host.thins))
        require(
            quiet.compositor.identity == combined.compositor.identity,
            "combined deploy caused a late second compositor restart",
        )
        return combined, trace

    def _crash_respawn(
        self,
        dev: Device,
        evidence: Evidence,
        baseline: Snapshot,
        combined: Snapshot,
    ) -> None:
        sampler = _PidSampler(dev)
        sampler.start()
        try:
            dev.run(f"kill -KILL {combined.compositor.pid}")
            crashed = self._snapshot(dev, len(baseline.thins))
        finally:
            sampler.stop()
        trace = _complete_trace(sampler.samples, crashed.compositor.pid)
        evidence.snapshot("crash-respawn", crashed)
        evidence.trace("crash-respawn", trace)
        evidence.text("crash-logread.txt", dev.read("logread | tail -n 200") + "\n")
        self._require_inventory(baseline, crashed)
        require(
            _classify_trace(combined.compositor.pid, trace) == _Transition.ONE,
            "crash did not observe exactly one procd respawn",
        )
        require(
            crashed.host.identity != combined.host.identity,
            "crash retained the old host",
        )
        require(
            crashed.host.starttime >= crashed.compositor.starttime,
            "respawned wasm host predates its compositor",
        )
        evidence.passed("crash-respawn")

    def run(self) -> None:
        repo = Path(_real_git(Path.cwd(), ("rev-parse", "--show-toplevel")))
        _require_tracked_clean(_real_git(repo, _TRACKED_STATUS))
        variants = _VariantWorktrees(repo, _TMP / "worktrees")
        paths = variants.create()
        dev = Device(self.device)
        evidence = Evidence(_TMP / time.strftime("run-%Y%m%d-%H%M%S"))
        touched = False

        def scenarios() -> None:
            nonlocal touched
            console.header("Widget host lifecycle E2E")
            dev.print()

            generation = _profile_generation(dev.read)
            pre_feature = self._snapshot(dev)
            evidence.snapshot("pre-feature", pre_feature)
            touched = True
            baseline = self._baseline(dev, repo, evidence, pre_feature)
            _complete_baseline(generation, pre_feature, baseline, evidence.passed)
            widget = self._widget_only(dev, paths["widget"], evidence, baseline)
            host = self._host_only(dev, paths["host"], evidence, baseline, widget)
            combined, combined_trace = self._combined(dev, paths["combined"], widget, host)
            evidence.snapshot("combined", combined)
            evidence.trace("combined", combined_trace)
            evidence.passed("combined")
            self._crash_respawn(dev, evidence, baseline, combined)

        def recover() -> None:
            if not touched:
                return
            self._deploy(repo, ("core", "widget-blockheight"))
            recovered = self._snapshot(dev)
            evidence.snapshot("recovered", recovered)
            evidence.passed("recovery")

        _finish(scenarios, recover, variants.cleanup)
        console.ok(f"evidence: {evidence.root}")


@entrypoint
def main(args: WidgetHostE2e) -> None:
    args.run()


if __name__ == "__main__":
    main()

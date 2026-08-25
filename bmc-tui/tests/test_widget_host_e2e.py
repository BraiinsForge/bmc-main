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

from collections.abc import Iterator, Sequence
from dataclasses import replace
from pathlib import Path

import pytest

from bmc_tui.procedures import widget_host_e2e as e2e
from bmc_tui.stage import Abort


def _thin(pid: int, starttime: int, widget: str, store: str) -> str:
    wasm = f"/nix/store/{store}-bmc-widget-{widget}/lib/bmc-widgets/{widget}/lib/wasm/{widget}.wasm"
    return (
        f"bmc-wasm-thin\t{pid}\t{starttime}\t/nix/store/thin\t"
        f"/nix/store/thin --wasm {wasm} --host-bin /nix/store/host"
    )


_HEALTHY = "\n".join(
    (
        "bmc-openwrt\t10\t100\t/nix/store/compositor\t/nix/store/compositor",
        "bmc-wasm-host\t20\t110\t/nix/store/host\t/nix/store/host",
        _thin(30, 120, "blockheight", "aaa"),
        _thin(31, 121, "clock", "bbb"),
        "",
    )
)


def _clock(values: Sequence[float]) -> Iterator[float]:
    return iter(values)


def test_snapshot_parses_typed_processes_and_selects_blockheight() -> None:
    snapshot = e2e.parse_snapshot(_HEALTHY)
    assert snapshot.compositor.identity == (10, 100)
    assert snapshot.host.identity == (20, 110)
    assert [thin.widget for thin in snapshot.thins] == ["blockheight", "clock"]
    assert [thin.pid for thin in snapshot.blockheight] == [30]
    assert snapshot.blockheight[0].wasm == (
        "/nix/store/aaa-bmc-widget-blockheight/lib/bmc-widgets/"
        "blockheight/lib/wasm/blockheight.wasm"
    )


@pytest.mark.parametrize(
    "raw, message",
    [
        ("bmc-openwrt\tbad\t1\t/x\t/x", "malformed process row"),
        (_HEALTHY.replace("bmc-wasm-host", "bmc-openwrt"), "expected one compositor"),
        (_HEALTHY.replace("bmc-wasm-thin", "unknown", 1), "malformed process row"),
    ],
)
def test_snapshot_rejects_malformed_or_unhealthy_process_sets(raw: str, message: str) -> None:
    with pytest.raises(ValueError, match=message):
        e2e.parse_snapshot(raw)


def test_health_deadline_retries_until_expected_inventory_arrives() -> None:
    snapshots = iter(["", _HEALTHY])
    clock = _clock([0, 0.1])
    result = e2e._wait_healthy(
        lambda: next(snapshots),
        timeout=1,
        expected_thins=2,
        sleep=lambda _delay: None,
        clock=lambda: next(clock),
    )
    assert result.compositor.pid == 10


def test_health_deadline_reports_last_failure() -> None:
    clock = _clock([0, 1])
    with pytest.raises(Abort, match="expected one compositor"):
        e2e._wait_healthy(
            lambda: "",
            timeout=1,
            sleep=lambda _delay: None,
            clock=lambda: next(clock),
        )


def test_pid_samples_preserve_every_pid_in_a_sample() -> None:
    assert e2e._parse_pid_sample("10 11") == frozenset({10, 11})
    assert e2e._parse_pid_sample("-") == frozenset()


def test_pid_trace_classifies_no_transition_and_one_restart() -> None:
    assert e2e._classify_trace(10, [frozenset({10}), frozenset()]) == e2e._Transition.NONE
    assert (
        e2e._classify_trace(10, [frozenset({10}), frozenset({10, 11}), frozenset({11})])
        == e2e._Transition.ONE
    )


def test_completed_pid_trace_includes_endpoint_and_requires_sampler_evidence() -> None:
    trace = e2e._complete_trace((frozenset({10}),), 11)
    assert trace == (frozenset({10}), frozenset({11}))
    assert e2e._classify_trace(10, trace) == e2e._Transition.ONE

    with pytest.raises(Abort, match="sampler produced no samples"):
        e2e._complete_trace((), 11)


def test_completed_pid_trace_rejects_an_observed_third_compositor() -> None:
    trace = e2e._complete_trace((frozenset({10}), frozenset({11})), 12)
    with pytest.raises(Abort, match="more than one"):
        e2e._classify_trace(10, trace)


def test_sampler_interval_is_accepted_by_integer_only_busybox_sleep() -> None:
    assert e2e._SAMPLE_INTERVAL_SECONDS == 1


def test_sampler_uses_canonical_live_compositor_pid() -> None:
    command = e2e._sampler_command("/tmp/marker")
    assert "cat /var/run/bmc-compositor.pid" in command
    assert "pidof" not in command
    assert 'readlink "/proc/$p/exe"' in command
    assert "*/bin/bmc-openwrt" in command
    assert "sleep 1" in command


def test_tracked_clean_gate_accepts_empty_status_and_rejects_changes() -> None:
    assert e2e._TRACKED_STATUS == ("status", "--porcelain", "--untracked-files=no")
    e2e._require_tracked_clean("")
    with pytest.raises(Abort, match="tracked worktree"):
        e2e._require_tracked_clean(" M bmc/src/main.rs")


def test_legacy_baseline_requires_host_replacement() -> None:
    snapshot = e2e.parse_snapshot(_HEALTHY)
    with pytest.raises(Abort, match="legacy wasm host"):
        e2e._require_baseline_transition(e2e._ProfileGeneration.LEGACY, snapshot, snapshot)


def test_legacy_baseline_accepts_host_replacement() -> None:
    before = e2e.parse_snapshot(_HEALTHY)
    after = replace(before, host=replace(before.host, pid=21, starttime=130))
    e2e._require_baseline_transition(e2e._ProfileGeneration.LEGACY, before, after)


def test_profile_probe_strips_output_and_propagates_failure() -> None:
    commands: list[str] = []

    def current(command: str) -> str:
        commands.append(command)
        return "yes\n"

    assert e2e._profile_generation(current) == e2e._ProfileGeneration.CURRENT
    assert commands == [
        "if test -x /run/current-profile/core/activation/scripts/"
        "999-signal-widget-reload; then echo yes; else echo no; fi"
    ]
    assert e2e._profile_generation(lambda _command: "no") == e2e._ProfileGeneration.LEGACY

    def fail(_command: str) -> str:
        raise RuntimeError("probe failed")

    with pytest.raises(RuntimeError, match="probe failed"):
        e2e._profile_generation(fail)
    with pytest.raises(Abort, match="unexpected widget reload hook probe"):
        e2e._profile_generation(lambda _command: "")


@pytest.mark.parametrize("replace_host", [False, True])
def test_current_baseline_places_no_constraint_on_host_identity(replace_host: bool) -> None:
    before = e2e.parse_snapshot(_HEALTHY)
    after = (
        replace(before, host=replace(before.host, pid=21, starttime=130))
        if replace_host
        else before
    )
    e2e._require_baseline_transition(e2e._ProfileGeneration.CURRENT, before, after)


def test_current_probe_records_baseline_pass_for_unchanged_host() -> None:
    snapshot = e2e.parse_snapshot(_HEALTHY)
    passed: list[str] = []
    generation = e2e._profile_generation(lambda _command: "yes\n")
    e2e._complete_baseline(generation, snapshot, snapshot, passed.append)
    assert passed == ["baseline"]


def test_legacy_probe_rejects_unchanged_host_before_recording_pass() -> None:
    snapshot = e2e.parse_snapshot(_HEALTHY)
    passed: list[str] = []
    generation = e2e._profile_generation(lambda _command: "no")
    with pytest.raises(Abort, match="legacy wasm host"):
        e2e._complete_baseline(generation, snapshot, snapshot, passed.append)
    assert passed == []


def test_run_probes_legacy_profile_before_baseline_deploy(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    events: list[str] = []
    passed: list[str] = []

    class FakeDevice:
        def __init__(self, _device: str) -> None:
            pass

        def print(self) -> None:
            pass

        def read(self, command: str) -> str:
            if command.startswith("if test -x "):
                events.append("probe")
                return "no"
            assert command == e2e.SNAPSHOT_COMMAND
            events.append("snapshot")
            return _HEALTHY

    class FakeEvidence:
        def __init__(self, root: Path) -> None:
            self.root = root

        def snapshot(self, name: str, _snapshot: e2e.Snapshot) -> None:
            events.append(f"snapshot:{name}")

        def passed(self, scenario: str) -> None:
            passed.append(scenario)

    class FakeVariants:
        def __init__(self, _repo: Path, root: Path) -> None:
            self.root = root

        def create(self) -> dict[str, Path]:
            return {variant.name: self.root / variant.name for variant in e2e._VARIANTS}

        def cleanup(self) -> None:
            events.append("cleanup")

    def git(_cwd: Path, args: Sequence[str]) -> str:
        return str(tmp_path) if tuple(args) == ("rev-parse", "--show-toplevel") else ""

    def deploy(_procedure: e2e.WidgetHostE2e, _cwd: Path, _packages: Sequence[str]) -> None:
        events.append("deploy")

    monkeypatch.setattr(e2e, "Device", FakeDevice)
    monkeypatch.setattr(e2e, "Evidence", FakeEvidence)
    monkeypatch.setattr(e2e, "_VariantWorktrees", FakeVariants)
    monkeypatch.setattr(e2e, "_real_git", git)
    monkeypatch.setattr(e2e.WidgetHostE2e, "_deploy", deploy)

    with pytest.raises(Abort, match="legacy wasm host"):
        e2e.WidgetHostE2e(device="deck").run()

    assert events.index("probe") < events.index("deploy")
    assert "baseline" not in passed


def test_lfs_guard_distinguishes_materialized_binary_from_pointer(tmp_path: Path) -> None:
    pointer = tmp_path / "pointer"
    binary = tmp_path / "binary"
    contents = {
        pointer: b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 3\n",
        binary: b"\x89PNG\r\n",
    }
    assert e2e._unresolved_lfs([pointer, binary], contents.__getitem__) == [pointer]


def test_variant_package_selection_is_pinned() -> None:
    assert {variant.name: variant.packages for variant in e2e._VARIANTS} == {
        "widget": ("widget-blockheight",),
        "host": ("core",),
        "combined": ("core", "widget-blockheight"),
    }


def test_variant_creation_cleans_created_worktree_when_edit_fails(tmp_path: Path) -> None:
    calls: list[tuple[str, ...]] = []

    def git(_cwd: Path, args: Sequence[str]) -> str:
        calls.append(tuple(args))
        if args[:3] == ("worktree", "add", "--detach"):
            Path(args[3]).mkdir(parents=True)
        return ""

    def fail_edit(_path: Path, _content: str) -> None:
        raise RuntimeError("edit failed")

    worktrees = e2e._VariantWorktrees(
        tmp_path / "repo", tmp_path / "variants", git=git, append=fail_edit
    )
    with pytest.raises(RuntimeError, match="edit failed"):
        worktrees.create()
    assert any(args[:3] == ("worktree", "remove", "--force") for args in calls)


def test_recovery_precedes_cleanup_after_scenario_failure() -> None:
    calls: list[str] = []

    def fail() -> None:
        calls.append("scenario")
        raise RuntimeError("failed")

    with pytest.raises(RuntimeError, match="failed"):
        e2e._finish(
            fail,
            lambda: calls.append("recovery"),
            lambda: calls.append("cleanup"),
        )
    assert calls == ["scenario", "recovery", "cleanup"]

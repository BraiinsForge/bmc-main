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

import subprocess
from dataclasses import replace
from pathlib import Path
from typing import TYPE_CHECKING, cast

import pytest

from bmc_tui.procedures import widget_host_e2e as host_e2e
from bmc_tui.procedures import widget_restart_e2e as e2e
from bmc_tui.stage import Abort

if TYPE_CHECKING:
    from bmc_tui.device import Device


class _Device:
    def __init__(self, *, read: str | tuple[str, ...] = "", run: str | None = "") -> None:
        self.read_results = iter((read,) if isinstance(read, str) else read)
        self.run_result = run
        self.read_commands: list[str] = []
        self.run_commands: list[str] = []

    def read(self, command: str) -> str:
        self.read_commands.append(command)
        return next(self.read_results)

    def run(self, command: str) -> str | None:
        self.run_commands.append(command)
        return self.run_result


def _thin(pid: int, starttime: int, widget: str) -> host_e2e.Process:
    raw = (
        "bmc-openwrt\t10\t100\t/nix/store/compositor\t/nix/store/compositor\n"
        "bmc-wasm-host\t20\t110\t/nix/store/host\t/nix/store/host\n"
        f"bmc-wasm-thin\t{pid}\t{starttime}\t/nix/store/thin\t"
        f"/nix/store/thin --wasm /nix/store/aaa-bmc-widget-{widget}/lib/bmc-widgets/"
        f"{widget}/lib/wasm/{widget}.wasm\n"
    )
    return host_e2e.parse_snapshot(raw).thins[0]


def _snapshot(thins: tuple[host_e2e.Process, ...]) -> host_e2e.Snapshot:
    raw = (
        "bmc-openwrt\t10\t100\t/nix/store/compositor\t/nix/store/compositor\n"
        "bmc-wasm-host\t20\t110\t/nix/store/host\t/nix/store/host\n"
    )
    base = host_e2e.parse_snapshot(raw + "bmc-wasm-thin\t1\t1\t/x\t/x --wasm /x\n")
    return replace(base, thins=thins)


def _raw(snapshot: host_e2e.Snapshot) -> str:
    return "\n".join(
        "\t".join(
            (
                str(process.role),
                str(process.pid),
                str(process.starttime),
                process.executable,
                " ".join(process.cmdline),
            )
        )
        for process in (snapshot.compositor, snapshot.host, *snapshot.thins)
    )


def test_thin_recovery_replaces_only_the_killed_process() -> None:
    clock = _thin(30, 120, "clock")
    weather = _thin(31, 121, "weather")
    replacement = replace(clock, pid=32, starttime=130)
    e2e._require_thin_recovered(
        _snapshot((clock, weather)), clock, _snapshot((replacement, weather))
    )


def test_thin_recovery_rejects_a_restarted_compositor() -> None:
    clock = _thin(30, 120, "clock")
    before = _snapshot((clock,))
    after = replace(
        _snapshot((replace(clock, pid=31, starttime=130),)),
        compositor=replace(before.compositor, pid=11, starttime=125),
    )
    with pytest.raises(Abort, match="thin crash restarted compositor"):
        e2e._require_thin_recovered(before, clock, after)


def test_thin_recovery_rejects_replacing_an_untargeted_widget() -> None:
    clock = _thin(30, 120, "clock")
    weather = _thin(31, 121, "weather")
    after = _snapshot(
        (
            replace(clock, pid=32, starttime=130),
            replace(weather, pid=33, starttime=131),
        )
    )
    with pytest.raises(Abort, match="replaced an untargeted widget"):
        e2e._require_thin_recovered(_snapshot((clock, weather)), clock, after)


def test_thin_recovery_rejects_a_changed_wasm_target() -> None:
    clock = _thin(30, 120, "clock")
    changed = replace(
        clock,
        pid=31,
        starttime=130,
        cmdline=tuple(part.replace("/clock/", "/other/") for part in clock.cmdline),
    )
    with pytest.raises(Abort, match="thin crash changed wasm targets"):
        e2e._require_thin_recovered(_snapshot((clock,)), clock, _snapshot((changed,)))


def test_host_recovery_replaces_host_and_every_thin() -> None:
    before = _snapshot((_thin(30, 120, "clock"), _thin(31, 121, "weather")))
    after = replace(
        _snapshot((_thin(32, 130, "clock"), _thin(33, 131, "weather"))),
        host=replace(before.host, pid=21, starttime=125),
    )
    e2e._require_host_recovered(before, after)


def test_host_recovery_rejects_changed_widget_inventory() -> None:
    before = _snapshot((_thin(30, 120, "clock"),))
    after = replace(
        _snapshot((_thin(31, 130, "weather"),)),
        host=replace(before.host, pid=21, starttime=125),
    )
    with pytest.raises(Abort, match="host crash changed widget inventory"):
        e2e._require_host_recovered(before, after)


def test_host_recovery_rejects_a_surviving_old_thin() -> None:
    clock = _thin(30, 120, "clock")
    before = _snapshot((clock,))
    after = replace(_snapshot((clock,)), host=replace(before.host, pid=21, starttime=125))
    with pytest.raises(Abort, match="host crash retained an old widget process"):
        e2e._require_host_recovered(before, after)


def test_log_evidence_correlates_exit_pid_with_compositor_connection() -> None:
    log = "\n".join(
        (
            "widget 11111111-1111-1111-1111-111111111111 (pid=30) exited: signal: 9",
            "Widget disconnected: 11111111-1111-1111-1111-111111111111",
            "Widget connected: 11111111-1111-1111-1111-111111111111",
        )
    )
    assert e2e._has_reconnection_evidence(log, {30})
    assert not e2e._has_reconnection_evidence(log, {31})


def test_log_evidence_rejects_a_connection_before_the_exit() -> None:
    log = "\n".join(
        (
            "Widget connected: 11111111-1111-1111-1111-111111111111",
            "widget 11111111-1111-1111-1111-111111111111 (pid=30) exited: signal: 9",
        )
    )
    assert not e2e._has_reconnection_evidence(log, {30})


def test_log_evidence_rejects_a_disconnect_after_reconnection() -> None:
    log = "\n".join(
        (
            "widget 11111111-1111-1111-1111-111111111111 (pid=30) exited: signal: 9",
            "Widget connected: 11111111-1111-1111-1111-111111111111",
            "Widget disconnected: 11111111-1111-1111-1111-111111111111",
        )
    )
    assert not e2e._has_reconnection_evidence(log, {30})


def test_baseline_requires_every_live_thin_to_connect_after_its_spawn() -> None:
    snapshot = _snapshot((_thin(30, 120, "clock"), _thin(31, 121, "weather")))
    log = "\n".join(
        (
            "widget instance 11111111-1111-1111-1111-111111111111 spawned (pid=30)",
            "Widget connected: 11111111-1111-1111-1111-111111111111",
            "widget instance 22222222-2222-2222-2222-222222222222 spawned (pid=31)",
            "Widget connected: 22222222-2222-2222-2222-222222222222",
        )
    )
    e2e._require_live_thins_connected(snapshot, log)


def test_baseline_rejects_a_live_thin_without_compositor_connection() -> None:
    snapshot = _snapshot((_thin(30, 120, "clock"),))
    log = "widget instance 11111111-1111-1111-1111-111111111111 spawned (pid=30)"
    with pytest.raises(Abort, match="lack compositor connection logs"):
        e2e._require_live_thins_connected(snapshot, log)


@pytest.mark.parametrize(
    "log",
    (
        "\n".join(
            (
                "Widget connected: 11111111-1111-1111-1111-111111111111",
                "widget instance 11111111-1111-1111-1111-111111111111 spawned (pid=30)",
            )
        ),
        "\n".join(
            (
                "widget instance 11111111-1111-1111-1111-111111111111 spawned (pid=30)",
                "Widget connected: 11111111-1111-1111-1111-111111111111",
                "Widget disconnected: 11111111-1111-1111-1111-111111111111",
            )
        ),
    ),
)
def test_baseline_rejects_a_thin_without_a_current_connection(log: str) -> None:
    with pytest.raises(Abort, match="lack compositor connection logs"):
        e2e._require_live_thins_connected(_snapshot((_thin(30, 120, "clock"),)), log)


def test_baseline_rejects_a_spawn_from_an_exited_pid_era() -> None:
    log = "\n".join(
        (
            "widget instance 11111111-1111-1111-1111-111111111111 spawned (pid=30)",
            "Widget connected: 11111111-1111-1111-1111-111111111111",
            "widget 11111111-1111-1111-1111-111111111111 (pid=30) exited: signal: 9",
        )
    )
    with pytest.raises(Abort, match="no retained spawn log"):
        e2e._require_live_thins_connected(_snapshot((_thin(30, 120, "clock"),)), log)


def test_baseline_waits_for_the_compositor_connection() -> None:
    snapshot = _snapshot((_thin(30, 120, "clock"),))
    spawn = "widget instance 11111111-1111-1111-1111-111111111111 spawned (pid=30)"
    connected = "Widget connected: 11111111-1111-1111-1111-111111111111"
    fake = _Device(read=(spawn, _raw(snapshot), f"{spawn}\n{connected}", _raw(snapshot)))
    result, log = e2e._wait_for_baseline(
        cast("Device", fake),
        sleep=lambda _delay: None,
        clock=iter((0.0, 0.0)).__next__,
    )
    assert result == snapshot
    assert log.endswith(connected)


def test_baseline_does_not_retry_missing_retained_spawn_history() -> None:
    fake = _Device(read=("", _raw(_snapshot((_thin(30, 120, "clock"),)))))
    with pytest.raises(Abort, match="no retained spawn log"):
        e2e._wait_for_baseline(cast("Device", fake), clock=lambda: 0.0)
    assert len(fake.read_commands) == 2


def test_log_delta_reads_after_marker_across_rotated_history() -> None:
    marker = f"{e2e._LOG_MARKER} nonce"
    fake = _Device(read=("", f"{marker}\nreconnected"))
    result = e2e._log_delta(cast("Device", fake), e2e._LogCursor(nonce="nonce"))
    assert result == "reconnected"
    assert "cat /var/log/bmc/bmc.log" in fake.read_commands[0]
    archives = fake.read_commands[1]
    assert archives.index("bmc.log.9") < archives.index("bmc.log.1")
    assert 'zcat "$archive.gz"' in archives
    assert 'cat "$archive"' in archives


def test_log_delta_caches_the_rotated_prefix() -> None:
    marker = f"{e2e._LOG_MARKER} nonce"
    fake = _Device(read=("", f"{marker}\narchived", "live"))
    cursor = e2e._LogCursor(nonce="nonce")
    assert e2e._log_delta(cast("Device", fake), cursor) == "archived"
    assert e2e._log_delta(cast("Device", fake), cursor) == "archived\nlive"
    assert len(fake.read_commands) == 3


def test_log_delta_rescans_after_another_rotation() -> None:
    marker = f"{e2e._LOG_MARKER} nonce"
    fake = _Device(
        read=(
            "",
            f"{marker}\narchived",
            "live-one",
            "live-two",
            f"{marker}\narchived\nlive-one",
        )
    )
    cursor = e2e._LogCursor(nonce="nonce")
    assert e2e._log_delta(cast("Device", fake), cursor) == "archived"
    assert e2e._log_delta(cast("Device", fake), cursor) == "archived\nlive-one"
    assert e2e._log_delta(cast("Device", fake), cursor) == "archived\nlive-one\nlive-two"
    assert len(fake.read_commands) == 5


def test_log_delta_reports_a_missing_marker() -> None:
    fake = _Device(read=("", ""))
    result = e2e._log_delta(cast("Device", fake), e2e._LogCursor(nonce="nonce"))
    assert result == e2e._LOG_HISTORY_LOST


def test_log_delta_reports_missing_file_logging() -> None:
    fake = _Device(read=e2e._LOG_UNAVAILABLE)
    with pytest.raises(Abort, match="became unavailable"):
        e2e._log_delta(cast("Device", fake), e2e._LogCursor(nonce="nonce"))


def test_log_delta_reports_archive_decompression_failure() -> None:
    failure = f"{e2e._LOG_ARCHIVE_FAILED} /var/log/bmc/bmc.log.1.gz"
    fake = _Device(read=("", failure))
    result = e2e._log_delta(cast("Device", fake), e2e._LogCursor(nonce="nonce"))
    assert result == failure


def test_log_history_reads_archives_oldest_first() -> None:
    fake = _Device(read="history")
    assert e2e._log_history(cast("Device", fake)) == "history"
    command = fake.read_commands[0]
    assert command.index("bmc.log.9") < command.index("bmc.log.1")
    assert command.index("bmc.log.1") < command.index("cat /var/log/bmc/bmc.log")
    assert 'zcat "$archive.gz"' in command
    assert 'cat "$archive"' in command


def test_log_history_reports_archive_decompression_failure() -> None:
    failure = f"{e2e._LOG_ARCHIVE_FAILED} /var/log/bmc/bmc.log.1.gz"
    fake = _Device(read=failure)
    with pytest.raises(Abort, match=r"failed to decompress /var/log/bmc/bmc\.log\.1\.gz"):
        e2e._log_history(cast("Device", fake))


def test_log_wait_retries_while_rotated_history_is_in_flight() -> None:
    log = "\n".join(
        (
            "widget 11111111-1111-1111-1111-111111111111 (pid=30) exited: signal: 9",
            "Widget connected: 11111111-1111-1111-1111-111111111111",
        )
    )
    reads = iter((e2e._LOG_HISTORY_LOST, log))
    result = e2e._wait_for_log(
        lambda: next(reads),
        {30},
        timeout=1,
        sleep=lambda _delay: None,
        clock=iter((0.0, 0.0)).__next__,
    )
    assert result == log


def test_log_wait_stops_after_repeated_history_loss() -> None:
    with pytest.raises(Abort, match=r"lost .* recovery history"):
        e2e._wait_for_log(
            lambda: e2e._LOG_HISTORY_LOST,
            {30},
            timeout=10,
            sleep=lambda _delay: None,
            clock=iter((0.0, 0.0)).__next__,
        )


def test_log_wait_clears_a_transient_history_failure() -> None:
    reads = iter((e2e._LOG_HISTORY_LOST, ""))
    with pytest.raises(Abort, match="no compositor reconnection evidence"):
        e2e._wait_for_log(
            lambda: next(reads),
            {30},
            timeout=1,
            sleep=lambda _delay: None,
            clock=iter((0.0, 0.0, 1.0)).__next__,
        )


def test_log_wait_retries_a_transient_ssh_failure() -> None:
    log = "\n".join(
        (
            "widget 11111111-1111-1111-1111-111111111111 (pid=30) exited: signal: 9",
            "Widget connected: 11111111-1111-1111-1111-111111111111",
        )
    )
    reads: list[subprocess.CalledProcessError | str] = [
        subprocess.CalledProcessError(255, "ssh"),
        log,
    ]

    def read() -> str:
        result = reads.pop(0)
        if isinstance(result, subprocess.CalledProcessError):
            raise result
        return result

    assert (
        e2e._wait_for_log(
            read,
            {30},
            timeout=1,
            sleep=lambda _delay: None,
            clock=iter((0.0, 0.0)).__next__,
        )
        == log
    )


def test_log_cursor_reports_missing_file_logging() -> None:
    dev = cast("Device", _Device(run=e2e._LOG_UNAVAILABLE))
    with pytest.raises(Abort, match="enable file logging"):
        e2e._log_cursor(dev)


def test_log_cursor_writes_a_unique_window_marker() -> None:
    fake = _Device()
    cursor = e2e._log_cursor(cast("Device", fake))
    assert cursor.nonce
    assert f"{e2e._LOG_MARKER} {cursor.nonce}" in fake.run_commands[0]


def test_failure_capture_keeps_log_when_snapshot_read_fails(tmp_path: Path) -> None:
    marker = f"{e2e._LOG_MARKER} nonce"

    class SnapshotFailure:
        def read(self, command: str) -> str:
            if command == host_e2e.SNAPSHOT_COMMAND:
                raise subprocess.CalledProcessError(1, command)
            return f"{marker}\nrecovery log"

    evidence = host_e2e.Evidence(tmp_path)
    e2e._record_failure(
        evidence,
        "thin",
        cast("Device", SnapshotFailure()),
        e2e._LogCursor(nonce="nonce"),
    )
    assert not (tmp_path / "thin-snapshot.txt").exists()
    assert (tmp_path / "thin.log").read_text() == "recovery log"


def test_kill_rejects_a_changed_process_identity() -> None:
    fake = _Device(run="PROCESS_IDENTITY_CHANGED")
    dev = cast("Device", fake)
    with pytest.raises(Abort, match="process 30 changed before kill"):
        e2e._kill(dev, _thin(30, 120, "clock"))
    assert "/proc/30/stat" in fake.run_commands[0]
    assert "sed 's/.*) //'" in fake.run_commands[0]
    assert "awk '{print $20}'" in fake.run_commands[0]
    assert '"$starttime" = 120' in fake.run_commands[0]


def test_log_wait_reports_missing_reconnection() -> None:
    clock = iter((0.0, 1.0))
    with pytest.raises(Abort, match="no compositor reconnection evidence"):
        e2e._wait_for_log(
            lambda: "",
            {30},
            timeout=1,
            sleep=lambda _delay: None,
            clock=lambda: next(clock),
        )


def test_transition_wait_ignores_healthy_pre_transition_snapshot() -> None:
    before = _snapshot((_thin(30, 120, "clock"),))
    after = _snapshot((_thin(31, 130, "clock"),))
    snapshots = iter((_raw(before), _raw(after)))
    result = e2e._wait_for_transition(
        lambda: next(snapshots),
        1,
        lambda snapshot: snapshot.thins[0].identity != before.thins[0].identity,
        sleep=lambda _delay: None,
        clock=iter((0.0, 0.0)).__next__,
    )
    assert result.thins[0].identity == after.thins[0].identity


def test_transition_wait_reports_the_current_failure() -> None:
    snapshot = _raw(_snapshot((_thin(30, 120, "clock"),)))
    reads = iter(("", snapshot))
    with pytest.raises(Abort, match="process transition not observed"):
        e2e._wait_for_transition(
            lambda: next(reads),
            1,
            lambda _snapshot: False,
            sleep=lambda _delay: None,
            clock=iter((0.0, 0.0, e2e._RECOVERY_TIMEOUT_SECONDS)).__next__,
        )


def test_transition_wait_rejects_a_partial_fleet() -> None:
    snapshot = _raw(_snapshot((_thin(30, 120, "clock"),)))
    with pytest.raises(Abort, match="expected 2 wasm thins, found 1"):
        e2e._wait_for_transition(
            lambda: snapshot,
            2,
            lambda _snapshot: True,
            sleep=lambda _delay: None,
            clock=iter((0.0, e2e._RECOVERY_TIMEOUT_SECONDS)).__next__,
        )

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

"""Validate widget and wasm-host crash recovery on a real Deck."""

import re
import subprocess
import tempfile
import time
import uuid
from collections import Counter
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from bmc_tui import console
from bmc_tui.device import Device
from bmc_tui.procedures.widget_host_e2e import (
    PROCESS_STARTTIME_FIELD_AFTER_COMM,
    SNAPSHOT_COMMAND,
    Evidence,
    Process,
    Snapshot,
    parse_snapshot,
)
from bmc_tui.stage import Abort, best_effort, entrypoint, require

_TMP = Path(tempfile.gettempdir()) / "bmc-widget-restart-e2e"
_BMC_LOG = "/var/log/bmc/bmc.log"
# Keep the E2E ceiling aligned with RESTART_BACKOFF_MAX and
# RESTART_HEALTHY_UPTIME in bmc/src/widget/manager.rs.
_RESTART_BACKOFF_MAX_SECONDS = 5 * 60
_RESTART_HEALTHY_UPTIME_SECONDS = 60
_RECOVERY_TIMEOUT_SECONDS = _RESTART_BACKOFF_MAX_SECONDS + _RESTART_HEALTHY_UPTIME_SECONDS
_POLL_INTERVAL_SECONDS = 1.0
_LOG_UNAVAILABLE = "BMC_LOG_UNAVAILABLE"
_LOG_HISTORY_LOST = "BMC_LOG_HISTORY_LOST"
_LOG_HISTORY_LOST_RETRY_LIMIT = 2
_LOG_ARCHIVE_FAILED = "BMC_LOG_ARCHIVE_FAILED"
_LOG_MARKER = "BMC_TUI_WIDGET_RESTART"
# Keep log coverage aligned with LOG_ROTATE_FILES_KEEP in bmc-log/src/lib.rs.
_LOG_ARCHIVES_KEPT = 9
_INSTANCE = r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
_EXIT = re.compile(rf"widget (?P<instance>{_INSTANCE}) \(pid=(?P<pid>[0-9]+)\) exited:")
_SPAWNED = re.compile(rf"widget instance (?P<instance>{_INSTANCE}) spawned \(pid=(?P<pid>[0-9]+)\)")
_CONNECTED = re.compile(rf"Widget connected: (?P<instance>{_INSTANCE})")
_DISCONNECTED = re.compile(rf"Widget disconnected: (?P<instance>{_INSTANCE})")


@dataclass
class _LogCursor:
    nonce: str
    archived_prefix: str | None = None
    live: str = ""


def _log_cursor(dev: Device) -> _LogCursor:
    cursor = _LogCursor(uuid.uuid4().hex)
    result = dev.run(
        f"if [ -f {_BMC_LOG} ]; then printf '\n{_LOG_MARKER} {cursor.nonce}\n' >> {_BMC_LOG}; "
        f"else echo {_LOG_UNAVAILABLE}; fi"
    )
    require(result != _LOG_UNAVAILABLE, f"{_BMC_LOG} is unavailable; enable file logging")
    return cursor


def _read_live_log(dev: Device) -> str:
    live = dev.read(f"if [ -f {_BMC_LOG} ]; then cat {_BMC_LOG}; else echo {_LOG_UNAVAILABLE}; fi")
    require(not live.startswith(_LOG_UNAVAILABLE), f"{_BMC_LOG} became unavailable")
    return live


def _archive_command() -> str:
    archives = " ".join(f"{_BMC_LOG}.{index}" for index in range(_LOG_ARCHIVES_KEPT, 0, -1))
    return (
        f"for archive in {archives}; do "
        f'if [ -f "$archive.gz" ]; then zcat "$archive.gz" || {{ echo "{_LOG_ARCHIVE_FAILED} '
        '$archive.gz"; break; }; '
        f'elif [ -f "$archive" ]; then cat "$archive" || {{ echo "{_LOG_ARCHIVE_FAILED} '
        '$archive"; break; }; fi; '
        "done"
    )


def _log_delta(dev: Device, cursor: _LogCursor) -> str:
    marker = f"{_LOG_MARKER} {cursor.nonce}"
    live = _read_live_log(dev)
    if marker in live:
        cursor.live = live
        return live.partition(marker)[2].lstrip("\n")
    if cursor.archived_prefix is not None and live.startswith(cursor.live):
        cursor.live = live
        return "\n".join(part for part in (cursor.archived_prefix, live) if part)
    archived = dev.read(_archive_command())
    if _LOG_ARCHIVE_FAILED in archived:
        return archived[archived.index(_LOG_ARCHIVE_FAILED) :].splitlines()[0]
    if marker not in archived:
        return _LOG_HISTORY_LOST
    cursor.archived_prefix = archived.partition(marker)[2].lstrip("\n")
    cursor.live = live
    return "\n".join(part for part in (cursor.archived_prefix, live) if part)


def _log_history(dev: Device) -> str:
    command = (
        f"if [ ! -f {_BMC_LOG} ]; then echo {_LOG_UNAVAILABLE}; else "
        f"{_archive_command()}; cat {_BMC_LOG}; fi"
    )
    history = dev.read(command)
    require(history != _LOG_UNAVAILABLE, f"{_BMC_LOG} is unavailable; enable file logging")
    if _LOG_ARCHIVE_FAILED in history:
        failure = history[history.index(_LOG_ARCHIVE_FAILED) :].splitlines()[0]
        raise Abort(failure.replace(_LOG_ARCHIVE_FAILED, "failed to decompress"))
    return history


@dataclass(frozen=True)
class _ConnectionEvents:
    connected: int | None = None
    disconnected: int | None = None


class _MissingSpawnHistory(Abort):
    pass


def _connection_events(log: str) -> dict[str, _ConnectionEvents]:
    events: dict[str, _ConnectionEvents] = {}
    for index, line in enumerate(log.splitlines()):
        if match := _CONNECTED.search(line):
            previous = events.get(match["instance"], _ConnectionEvents())
            events[match["instance"]] = _ConnectionEvents(index, previous.disconnected)
        if match := _DISCONNECTED.search(line):
            previous = events.get(match["instance"], _ConnectionEvents())
            events[match["instance"]] = _ConnectionEvents(previous.connected, index)
    return events


def _connected_after(events: _ConnectionEvents | None, boundary: int) -> bool:
    if events is None or events.connected is None or events.connected <= boundary:
        return False
    return events.disconnected is None or events.connected > events.disconnected


def _require_live_thins_connected(snapshot: Snapshot, log: str) -> None:
    spawned: dict[int, tuple[str, int]] = {}
    exited: dict[int, int] = {}
    for index, line in enumerate(log.splitlines()):
        if match := _SPAWNED.search(line):
            spawned[int(match["pid"])] = match["instance"], index
        if match := _EXIT.search(line):
            exited[int(match["pid"])] = index
    connections = _connection_events(log)

    live_pids = {thin.pid for thin in snapshot.thins}
    missing_spawns = {
        pid for pid in live_pids if pid not in spawned or exited.get(pid, -1) > spawned[pid][1]
    }
    if missing_spawns:
        raise _MissingSpawnHistory(
            f"no retained spawn log for live widget PIDs {sorted(missing_spawns)}; "
            "deploy or restart widgets before running this startup check"
        )
    disconnected = {
        pid
        for pid in live_pids
        if not _connected_after(connections.get(spawned[pid][0]), spawned[pid][1])
    }
    require(
        not disconnected,
        f"live widget PIDs lack compositor connection logs: {sorted(disconnected)}",
    )


def _wait_for_baseline(
    dev: Device,
    *,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> tuple[Snapshot, str]:
    deadline = clock() + _RECOVERY_TIMEOUT_SECONDS
    log = _log_history(dev)
    previous_live: str | None = None
    failure = "widget baseline not ready"
    while True:
        try:
            snapshot = parse_snapshot(dev.read(SNAPSHOT_COMMAND))
            _require_live_thins_connected(snapshot, log)
        except _MissingSpawnHistory:
            raise
        except Abort as error:
            failure = error.hint
        except (ValueError, subprocess.CalledProcessError) as error:
            failure = str(error)
        else:
            return snapshot, log
        if clock() >= deadline:
            raise Abort(f"widget baseline did not become ready: {failure}")
        sleep(_POLL_INTERVAL_SECONDS)
        live = _read_live_log(dev)
        if previous_live is None:
            log = "\n".join(part for part in (log, live) if part)
        elif live.startswith(previous_live):
            log += live[len(previous_live) :]
        else:
            log = _log_history(dev)
        previous_live = live


def _has_reconnection_evidence(log: str, pids: set[int]) -> bool:
    exited: dict[int, tuple[str, int]] = {}
    for index, line in enumerate(log.splitlines()):
        if match := _EXIT.search(line):
            exited[int(match["pid"])] = match["instance"], index
    connections = _connection_events(log)
    return pids <= exited.keys() and all(
        _connected_after(connections.get(exited[pid][0]), exited[pid][1]) for pid in pids
    )


def _wait_for_log(
    read: Callable[[], str],
    pids: set[int],
    *,
    timeout: float = _RECOVERY_TIMEOUT_SECONDS,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> str:
    deadline = clock() + timeout
    missing_reconnection = f"no compositor reconnection evidence for widget PIDs {sorted(pids)}"
    failure = missing_reconnection
    lost_reads = 0
    while True:
        try:
            log = read()
            if log == _LOG_HISTORY_LOST:
                lost_reads += 1
                failure = f"lost {_BMC_LOG} recovery history"
                if lost_reads >= _LOG_HISTORY_LOST_RETRY_LIMIT:
                    raise Abort(failure)
            elif log.startswith(_LOG_ARCHIVE_FAILED):
                lost_reads = 0
                failure = log.replace(_LOG_ARCHIVE_FAILED, "failed to decompress", 1)
            else:
                lost_reads = 0
                failure = missing_reconnection
                if _has_reconnection_evidence(log, pids):
                    return log
        except subprocess.CalledProcessError as error:
            failure = f"log probe failed: {error}"
        if clock() >= deadline:
            raise Abort(failure)
        sleep(_POLL_INTERVAL_SECONDS)


def _wait_for_transition(
    read: Callable[[], str],
    expected_thins: int,
    accept: Callable[[Snapshot], bool],
    *,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.monotonic,
) -> Snapshot:
    timeout = _RECOVERY_TIMEOUT_SECONDS
    deadline = clock() + timeout
    failure = "process transition not observed"
    while True:
        try:
            snapshot = parse_snapshot(read())
            failure = "process transition not observed"
            if len(snapshot.thins) != expected_thins:
                raise ValueError(
                    f"expected {expected_thins} wasm thins, found {len(snapshot.thins)}"
                )
            if accept(snapshot):
                return snapshot
        except (ValueError, subprocess.CalledProcessError) as error:
            failure = str(error)
        if clock() >= deadline:
            raise Abort(f"widget recovery did not complete within {timeout:g}s: {failure}")
        sleep(_POLL_INTERVAL_SECONDS)


def _kill(dev: Device, process: Process) -> None:
    result = dev.run(
        f"starttime=$(sed 's/.*) //' /proc/{process.pid}/stat 2>/dev/null | "
        f"awk '{{print ${PROCESS_STARTTIME_FIELD_AFTER_COMM}}}'); "
        f'if [ "$starttime" = {process.starttime} ]; then kill -KILL {process.pid}; '
        "else echo PROCESS_IDENTITY_CHANGED; fi"
    )
    require(result != "PROCESS_IDENTITY_CHANGED", f"process {process.pid} changed before kill")


def _record_failure(evidence: Evidence, name: str, dev: Device, cursor: _LogCursor) -> None:
    best_effort(lambda: evidence.text(f"{name}-snapshot.txt", dev.read(SNAPSHOT_COMMAND)))
    best_effort(lambda: evidence.text(f"{name}.log", _log_delta(dev, cursor)))


def _wasm_inventory(snapshot: Snapshot) -> Counter[str | None]:
    return Counter(thin.wasm for thin in snapshot.thins)


def _require_thin_recovered(before: Snapshot, killed: Process, after: Snapshot) -> None:
    require(
        after.compositor.identity == before.compositor.identity,
        "thin crash restarted compositor",
    )
    require(after.host.identity == before.host.identity, "thin crash replaced wasm host")
    require(after.inventory == before.inventory, "thin crash changed widget inventory")
    require(_wasm_inventory(after) == _wasm_inventory(before), "thin crash changed wasm targets")
    survivors = {thin.identity for thin in before.thins if thin.identity != killed.identity}
    current = {thin.identity for thin in after.thins}
    require(killed.identity not in current, "killed thin retained its process identity")
    require(survivors <= current, "thin crash replaced an untargeted widget")


def _require_host_recovered(before: Snapshot, after: Snapshot) -> None:
    require(
        after.compositor.identity == before.compositor.identity,
        "host crash restarted compositor",
    )
    require(after.host.identity != before.host.identity, "host crash retained the old host")
    require(after.inventory == before.inventory, "host crash changed widget inventory")
    require(_wasm_inventory(after) == _wasm_inventory(before), "host crash changed wasm targets")
    old_thins = {thin.identity for thin in before.thins}
    new_thins = {thin.identity for thin in after.thins}
    require(old_thins.isdisjoint(new_thins), "host crash retained an old widget process")


@dataclass
class WidgetRestartE2e:
    device: str  # IP or host of the target Deck

    def run(self) -> None:
        dev = Device(self.device)
        evidence = Evidence(_TMP / time.strftime("run-%Y%m%d-%H%M%S"))

        console.header("Widget restart E2E")
        dev.print()

        current, baseline_log = _wait_for_baseline(dev)
        evidence.snapshot("baseline", current)
        evidence.text("baseline.log", baseline_log)
        evidence.passed("baseline")

        baseline_thins = current.thins
        for index, killed in enumerate(baseline_thins, start=1):
            name = f"thin-{index}-{killed.widget or 'unknown'}"
            cursor = _log_cursor(dev)
            _kill(dev, killed)
            try:
                recovered = _wait_for_transition(
                    lambda: dev.read(SNAPSHOT_COMMAND),
                    len(current.thins),
                    lambda snapshot, killed=killed: (
                        killed.identity not in {thin.identity for thin in snapshot.thins}
                    ),
                )
                log = _wait_for_log(lambda cursor=cursor: _log_delta(dev, cursor), {killed.pid})
            except Abort:
                _record_failure(evidence, name, dev, cursor)
                raise
            evidence.snapshot(name, recovered)
            evidence.text(f"{name}.log", log)
            _require_thin_recovered(current, killed, recovered)
            evidence.passed(name)
            current = recovered

        cursor = _log_cursor(dev)
        old_pids = {thin.pid for thin in current.thins}
        old_identities = {thin.identity for thin in current.thins}
        _kill(dev, current.host)
        try:
            recovered = _wait_for_transition(
                lambda: dev.read(SNAPSHOT_COMMAND),
                len(current.thins),
                lambda snapshot: (
                    snapshot.host.identity != current.host.identity
                    and old_identities.isdisjoint(thin.identity for thin in snapshot.thins)
                ),
            )
            log = _wait_for_log(lambda: _log_delta(dev, cursor), old_pids)
        except Abort:
            _record_failure(evidence, "host", dev, cursor)
            raise
        evidence.snapshot("host", recovered)
        evidence.text("host.log", log)
        _require_host_recovered(current, recovered)
        evidence.passed("host")

        console.ok(f"evidence: {evidence.root}")


@entrypoint
def main(args: WidgetRestartE2e) -> None:
    args.run()


if __name__ == "__main__":
    main()

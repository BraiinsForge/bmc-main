"""VM metrics collection — snapshots of memory, CPU, load, and per-process RSS.

The actual /proc parsing lives in :mod:`bmc_virt.server` so the daemon can
read the files directly in Python instead of shelling out to busybox tools
(``pidof -s`` is not in every busybox build, and shelling per snapshot adds
non-trivial overhead at sub-second cadence).
"""

from __future__ import annotations

import contextlib
import pathlib
import threading
import time
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import TYPE_CHECKING, Any

import matplotlib
import matplotlib.pyplot as plt
from rich import box
from rich.table import Table

from bmc_virt import ui
from bmc_virt.commands import Cmd

matplotlib.use("Agg")

if TYPE_CHECKING:
    from bmc_virt.vm import VM

_MEM_WARN_PCT = 60
_MEM_CRIT_PCT = 80

# Per-process plot colors, cycled in order. Picked to contrast with the
# existing memory-used (#ff6b6b) and memory-avail (#51cf66) lines.
_PROC_COLORS = ("#ffd43b", "#845ef7", "#f06595", "#22b8cf", "#fd7e14")


@dataclass
class ProcSnapshot:
    """Per-process memory snapshot from /proc/<pid>/status.

    ``pid`` is ``None`` when the process was not running at sample time.
    All sizes are in kilobytes; missing fields default to 0.
    """

    pid: int | None
    vm_rss_kb: int = 0
    rss_anon_kb: int = 0
    rss_file_kb: int = 0
    rss_shmem_kb: int = 0
    vm_size_kb: int = 0


@dataclass
class Snapshot:
    """A single metrics snapshot."""

    ts: datetime
    label: str
    mem_total_kb: int
    mem_available_kb: int
    mem_used_kb: int
    load_1m: float
    load_5m: float
    load_15m: float
    uptime_s: float
    raw: dict[str, Any] = field(default_factory=dict)
    processes: dict[str, ProcSnapshot] = field(default_factory=dict)


class MetricsCollector:
    """Collects VM metrics snapshots over time.

    Without interval (default): purely imperative, each capture() polls once.
    With interval: a background thread polls automatically, close() stops it.

    Pass ``processes=["bmc-wasm-host", "bmc-openwrt"]`` to also sample each
    listed process's VmRSS / RssAnon / RssShmem from /proc/<pid>/status.

    Usage (imperative):
        m = vm.metrics.start("My test", processes=["bmc-wasm-host"])
        m.capture("before")
        # ... do stuff ...
        m.capture("after")
        m.report()

    Usage (auto-poll):
        m = vm.metrics.start("My test", interval=0.5)
        # ... do stuff, snapshots collected in background ...
        m.capture("named marker")  # still works — adds a labeled snapshot
        m.close()
        m.report()
    """

    def __init__(
        self,
        vm: VM,
        label: str = "",
        interval: float | None = None,
        *,
        processes: list[str] | None = None,
    ) -> None:
        self._vm = vm
        self.label = label
        self.processes = list(processes or [])
        self.snapshots: list[Snapshot] = []
        self._closed = False
        self._poll_thread: threading.Thread | None = None

        if interval is not None:
            self._poll_thread = threading.Thread(
                target=self._poll_loop,
                args=(interval,),
                daemon=True,
                name="metrics-poller",
            )
            self._poll_thread.start()

    def capture(self, label: str = "") -> Snapshot:
        """Take a metrics snapshot right now."""
        if self._closed:
            msg = "MetricsCollector is closed"
            raise RuntimeError(msg)
        return self._take_snapshot(label)

    def close(self) -> None:
        """Stop the background poller (no-op if not auto-polling)."""
        self._closed = True
        if self._poll_thread is not None:
            self._poll_thread.join(timeout=5)

    def __enter__(self) -> MetricsCollector:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def _take_snapshot(self, label: str = "") -> Snapshot:
        """Send ``metrics.collect`` to the daemon and record the response."""
        ack = self._vm._send_cmd(
            Cmd.METRICS_COLLECT,
            timeout=5,
            verbose=False,
            processes=self.processes,
        )
        if not ack.ok:
            msg = f"Failed to read metrics: {ack.error}"
            raise RuntimeError(msg)

        snapshot = _snapshot_from_ack(ack.data, label)
        self.snapshots.append(snapshot)
        return snapshot

    def _poll_loop(self, interval: float) -> None:
        """Background thread: poll metrics at a fixed interval."""
        while not self._closed:
            with contextlib.suppress(RuntimeError):
                self._take_snapshot()
            time.sleep(interval)

    def report(self) -> None:
        """Print a rich table of all snapshots."""

        table = Table(
            title=self.label or "Metrics",
            title_style="bold",
            title_justify="left",
            show_header=True,
            header_style="bold",
            box=box.HEAVY_HEAD,
            border_style="bright_black",
            padding=(0, 1),
        )
        table.add_column("Time", style="dim")
        table.add_column("Label")
        table.add_column("Used", justify="right")
        table.add_column("Avail", justify="right")
        table.add_column("Mem%", justify="right")
        table.add_column("1m", justify="right")
        table.add_column("5m", justify="right")
        table.add_column("15m", justify="right")

        for s in self.snapshots:
            mem_pct = (s.mem_used_kb / s.mem_total_kb * 100) if s.mem_total_kb else 0
            if mem_pct > _MEM_CRIT_PCT:
                pct_style = "red"
            elif mem_pct > _MEM_WARN_PCT:
                pct_style = "yellow"
            else:
                pct_style = "green"
            table.add_row(
                ui.format_ts(s.ts),
                s.label or "—",
                f"{s.mem_used_kb // 1024}M",
                f"{s.mem_available_kb // 1024}M",
                f"[{pct_style}]{mem_pct:.0f}%[/{pct_style}]",
                f"{s.load_1m:.2f}",
                f"{s.load_5m:.2f}",
                f"{s.load_15m:.2f}",
            )

        ui.out.print("")
        ui.out.print(table)

        if self.processes:
            self._report_processes()

    def _report_processes(self) -> None:
        """Print a per-process RSS table (only labeled snapshots)."""
        labeled = [s for s in self.snapshots if s.label]
        if not labeled:
            return

        table = Table(
            title=f"{self.label or 'Metrics'} — per-process RSS",
            title_style="bold",
            title_justify="left",
            show_header=True,
            header_style="bold",
            box=box.HEAVY_HEAD,
            border_style="bright_black",
            padding=(0, 1),
        )
        table.add_column("Time", style="dim")
        table.add_column("Label")
        for name in self.processes:
            table.add_column(name, justify="right")

        for s in labeled:
            row = [ui.format_ts(s.ts), s.label]
            for name in self.processes:
                proc = s.processes.get(name)
                if proc is None or proc.pid is None:
                    row.append("[red]missing[/red]")
                else:
                    row.append(f"{proc.vm_rss_kb // 1024}M")
            table.add_row(*row)

        ui.out.print("")
        ui.out.print(table)

    def chart(self, dest: str | pathlib.Path) -> pathlib.Path:
        """Generate a metrics chart as PNG. Dark theme, single plot with dual y-axis."""
        dest = pathlib.Path(dest)
        dest.parent.mkdir(parents=True, exist_ok=True)

        plt.style.use("dark_background")

        timestamps = [s.ts for s in self.snapshots]
        mem_used = [s.mem_used_kb / 1024 for s in self.snapshots]
        mem_avail = [s.mem_available_kb / 1024 for s in self.snapshots]
        load_1m = [s.load_1m for s in self.snapshots]

        fig, ax1 = plt.subplots(figsize=(12, 5))
        fig.suptitle(self.label or "VM Metrics", fontsize=14, fontweight="bold")

        # Memory on left y-axis
        ax1.set_ylabel("Memory (MB)", color="#ff6b6b")
        ax1.plot(timestamps, mem_used, "#ff6b6b", linewidth=2, label="Mem Used")  # ty: ignore[invalid-argument-type]  # matplotlib accepts datetime
        ax1.plot(timestamps, mem_avail, "#51cf66", linewidth=2, label="Mem Avail")  # ty: ignore[invalid-argument-type]
        ax1.tick_params(axis="y", labelcolor="#ff6b6b")
        ax1.set_xlabel("Time")
        ax1.grid(True, alpha=0.15)

        # Per-process VmRSS on the same axis. None values render as gaps.
        for i, name in enumerate(self.processes):
            color = _PROC_COLORS[i % len(_PROC_COLORS)]
            rss_series: list[float | None] = []
            for s in self.snapshots:
                proc = s.processes.get(name)
                if proc is None or proc.pid is None:
                    rss_series.append(None)
                else:
                    rss_series.append(proc.vm_rss_kb / 1024)
            if all(v is None for v in rss_series):
                continue
            ax1.plot(timestamps, rss_series, color, linewidth=1.5, label=f"{name} RSS")  # ty: ignore[invalid-argument-type]

        # Load on right y-axis
        ax2 = ax1.twinx()
        ax2.set_ylabel("Load Average", color="#74c0fc")
        ax2.plot(timestamps, load_1m, "#74c0fc", linewidth=2, linestyle="--", label="Load 1m")  # ty: ignore[invalid-argument-type]
        ax2.tick_params(axis="y", labelcolor="#74c0fc")

        # Combined legend
        lines1, labels1 = ax1.get_legend_handles_labels()
        lines2, labels2 = ax2.get_legend_handles_labels()
        ax1.legend(lines1 + lines2, labels1 + labels2, loc="upper left", framealpha=0.3)

        # Capture markers — placed below the chart as rotated x-axis
        # annotations. Density-aware staggering: rotate 30° and cycle four
        # vertical rows so dense prompt/ack pairs stay readable.
        labeled = [s for s in self.snapshots if s.label]
        stagger_rows = 4
        for i, s in enumerate(labeled):
            ax1.axvline(x=s.ts, color="#868e96", linestyle=":", alpha=0.6)  # ty: ignore[invalid-argument-type]
            y_pos = -0.10 - 0.06 * (i % stagger_rows)
            ax1.annotate(
                s.label,
                xy=(s.ts, 0),  # ty: ignore[invalid-argument-type]
                xycoords=("data", "axes fraction"),
                xytext=(0, y_pos * fig.get_figheight() * fig.dpi),
                textcoords="offset points",
                fontsize=8,
                color="#adb5bd",
                ha="right",
                rotation=30,
                rotation_mode="anchor",
            )

        fig.subplots_adjust(bottom=0.3)
        plt.savefig(dest, dpi=150, bbox_inches="tight", facecolor=fig.get_facecolor())
        plt.close()
        return dest


def _snapshot_from_ack(payload: dict[str, Any], label: str) -> Snapshot:
    """Build a :class:`Snapshot` from the daemon's ``metrics.collect`` ack.

    Tolerates partial payloads — missing sections fall back to zeroed
    values rather than raising, so a transient read error in the daemon
    yields a useful (if degraded) sample instead of dropping the whole row.
    """
    meminfo_raw = payload.get("meminfo") or {}
    meminfo: dict[str, int] = {str(k): int(v) for k, v in meminfo_raw.items()}

    load_raw = payload.get("loadavg") or [0.0, 0.0, 0.0]
    load = tuple(float(load_raw[i]) if i < len(load_raw) else 0.0 for i in range(3))
    load_1m, load_5m, load_15m = load

    uptime_s = float(payload.get("uptime_s", 0.0))

    processes_raw = payload.get("processes") or {}
    processes: dict[str, ProcSnapshot] = {
        str(name): _proc_snapshot_from_dict(rec) for name, rec in processes_raw.items()
    }

    mem_total = meminfo.get("MemTotal", 0)
    mem_available = meminfo.get("MemAvailable", 0)

    return Snapshot(
        ts=datetime.now(UTC),
        label=label,
        mem_total_kb=mem_total,
        mem_available_kb=mem_available,
        mem_used_kb=mem_total - mem_available,
        load_1m=load_1m,
        load_5m=load_5m,
        load_15m=load_15m,
        uptime_s=uptime_s,
        raw=meminfo,
        processes=processes,
    )


def _proc_snapshot_from_dict(rec: dict[str, Any]) -> ProcSnapshot:
    pid = rec.get("pid")
    return ProcSnapshot(
        pid=int(pid) if pid is not None else None,
        vm_rss_kb=int(rec.get("vm_rss_kb", 0)),
        rss_anon_kb=int(rec.get("rss_anon_kb", 0)),
        rss_file_kb=int(rec.get("rss_file_kb", 0)),
        rss_shmem_kb=int(rec.get("rss_shmem_kb", 0)),
        vm_size_kb=int(rec.get("vm_size_kb", 0)),
    )

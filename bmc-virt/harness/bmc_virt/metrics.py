"""VM metrics collection — snapshots of memory, CPU, and load."""

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

matplotlib.use("Agg")

if TYPE_CHECKING:
    from bmc_virt.vm import VM

_MEM_WARN_PCT = 60
_MEM_CRIT_PCT = 80
_LOADAVG_FIELDS = 3


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


class MetricsCollector:
    """Collects VM metrics snapshots over time.

    Without interval (default): purely imperative, each capture() polls once.
    With interval: a background thread polls automatically, close() stops it.

    Usage (imperative):
        m = vm.metrics.start("My test")
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

    def __init__(self, vm: VM, label: str = "", interval: float | None = None) -> None:
        self._vm = vm
        self.label = label
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
        """Internal: poll the VM and record a snapshot."""
        ack = self._vm.exec("cat /proc/meminfo /proc/loadavg /proc/uptime", timeout=5)
        if not ack.ok:
            msg = f"Failed to read metrics: {ack.error}"
            raise RuntimeError(msg)

        raw_output = ack.data.get("stdout", "")
        snapshot = _parse_snapshot(raw_output, label)
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

        # Load on right y-axis
        ax2 = ax1.twinx()
        ax2.set_ylabel("Load Average", color="#74c0fc")
        ax2.plot(timestamps, load_1m, "#74c0fc", linewidth=2, linestyle="--", label="Load 1m")  # ty: ignore[invalid-argument-type]
        ax2.tick_params(axis="y", labelcolor="#74c0fc")

        # Combined legend
        lines1, labels1 = ax1.get_legend_handles_labels()
        lines2, labels2 = ax2.get_legend_handles_labels()
        ax1.legend(lines1 + lines2, labels1 + labels2, loc="upper left", framealpha=0.3)

        # Capture markers — placed below the chart as x-axis annotations
        labeled = [s for s in self.snapshots if s.label]
        for i, s in enumerate(labeled):
            ax1.axvline(x=s.ts, color="#868e96", linestyle=":", alpha=0.6)  # ty: ignore[invalid-argument-type]
            # Stagger labels vertically to avoid overlap
            y_pos = -0.12 - 0.06 * (i % 2)
            ax1.annotate(
                s.label,
                xy=(s.ts, 0),  # ty: ignore[invalid-argument-type]
                xycoords=("data", "axes fraction"),
                xytext=(0, y_pos * fig.get_figheight() * fig.dpi),
                textcoords="offset points",
                fontsize=9,
                color="#adb5bd",
                ha="center",
            )

        fig.subplots_adjust(bottom=0.2)
        plt.savefig(dest, dpi=150, bbox_inches="tight", facecolor=fig.get_facecolor())
        plt.close()
        return dest


def _parse_snapshot(raw: str, label: str) -> Snapshot:
    """Parse combined output of /proc/meminfo + /proc/loadavg + /proc/uptime."""
    meminfo: dict[str, int] = {}
    load = (0.0, 0.0, 0.0)
    uptime = 0.0

    for raw_line in raw.splitlines():
        line = raw_line.strip()

        # /proc/meminfo lines: "MemTotal:       123456 kB"
        if ":" in line and "kB" in line:
            key, val = line.split(":", 1)
            meminfo[key.strip()] = int(val.strip().split()[0])

        # /proc/loadavg: "0.12 0.34 0.56 1/123 4567"
        elif line and line[0].isdigit() and "/" in line:
            parts = line.split()
            if len(parts) >= _LOADAVG_FIELDS:
                load = (float(parts[0]), float(parts[1]), float(parts[2]))

        # /proc/uptime: "12345.67 23456.78"
        elif line and line[0].isdigit() and "." in line and "/" not in line:
            parts = line.split()
            if parts:
                uptime = float(parts[0])

    mem_total = meminfo.get("MemTotal", 0)
    mem_available = meminfo.get("MemAvailable", 0)

    return Snapshot(
        ts=datetime.now(UTC),
        label=label,
        mem_total_kb=mem_total,
        mem_available_kb=mem_available,
        mem_used_kb=mem_total - mem_available,
        load_1m=load[0],
        load_5m=load[1],
        load_15m=load[2],
        uptime_s=uptime,
        raw=meminfo,
    )

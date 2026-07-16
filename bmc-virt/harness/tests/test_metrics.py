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

"""Unit tests for the metrics ack-unmarshal and the server-side /proc readers.

The /proc parsing now lives in :mod:`bmc_virt.server`, so its parsers are
exercised directly. The host-side :mod:`bmc_virt.metrics` only converts the
daemon's ack payload into a :class:`Snapshot`, which is tested against
canned dicts (no VM required).
"""

import os

from bmc_virt.metrics import ProcSnapshot, _snapshot_from_ack
from bmc_virt.server import (
    _read_loadavg,
    _read_meminfo,
    _read_proc_status,
    _read_uptime,
)

# ── Ack → Snapshot conversion ──────────────────────────────────────────────────


class TestSnapshotFromAck:
    def test_parses_full_payload(self) -> None:
        payload = {
            "meminfo": {
                "MemTotal": 262_144,
                "MemFree": 50_000,
                "MemAvailable": 100_000,
                "Shmem": 70_000,
            },
            "loadavg": [0.50, 0.40, 0.30],
            "uptime_s": 12_345.67,
            "processes": {
                "bmc-wasm-host": {
                    "pid": 3246,
                    "vm_size_kb": 108_504,
                    "vm_rss_kb": 22_980,
                    "rss_anon_kb": 9_920,
                    "rss_file_kb": 8,
                    "rss_shmem_kb": 13_052,
                },
            },
        }
        snap = _snapshot_from_ack(payload, "baseline")
        assert snap.label == "baseline"
        assert snap.mem_total_kb == 262_144
        assert snap.mem_available_kb == 100_000
        assert snap.mem_used_kb == 162_144
        assert (snap.load_1m, snap.load_5m, snap.load_15m) == (0.50, 0.40, 0.30)
        assert snap.uptime_s == 12_345.67
        assert snap.raw["Shmem"] == 70_000

        proc = snap.processes["bmc-wasm-host"]
        assert proc.pid == 3246
        assert proc.vm_rss_kb == 22_980
        assert proc.rss_shmem_kb == 13_052

    def test_missing_process_yields_none_pid(self) -> None:
        payload = {
            "meminfo": {"MemTotal": 1, "MemAvailable": 1},
            "loadavg": [0.0, 0.0, 0.0],
            "uptime_s": 0.0,
            "processes": {"bmc-wasm-host": {"pid": None}},
        }
        snap = _snapshot_from_ack(payload, "")
        assert snap.processes["bmc-wasm-host"] == ProcSnapshot(pid=None)

    def test_empty_payload_falls_back_to_zeros(self) -> None:
        snap = _snapshot_from_ack({}, "label")
        assert snap.label == "label"
        assert snap.mem_total_kb == 0
        assert snap.processes == {}
        assert snap.load_1m == 0.0
        assert snap.uptime_s == 0.0


# ── Server-side /proc readers ──────────────────────────────────────────────────


class TestReadMeminfo:
    def test_reads_real_meminfo(self) -> None:
        # /proc/meminfo always exists on Linux runners. Just check structure.
        m = _read_meminfo()
        assert m.get("MemTotal", 0) > 0
        assert "MemAvailable" in m


class TestReadLoadavg:
    def test_returns_three_floats(self) -> None:
        load = _read_loadavg()
        assert len(load) == 3
        assert all(isinstance(v, float) and v >= 0.0 for v in load)


class TestReadUptime:
    def test_uptime_is_positive(self) -> None:
        assert _read_uptime() > 0.0


class TestReadProcStatus:
    def test_self_status_has_vm_rss(self) -> None:
        rec = _read_proc_status(os.getpid())
        assert rec["pid"] == os.getpid()
        # Self should have nonzero RSS.
        assert rec["vm_rss_kb"] > 0

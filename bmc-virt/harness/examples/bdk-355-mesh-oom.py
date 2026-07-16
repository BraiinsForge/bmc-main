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

"""BDK-355 — mesh OOM repro and memory profile harness.

Drives the per-DieType memory experiment from the OOM analysis at
``docs/devlogs/BDK_355/hw-oom-on-second-mesh.md``. Each tap window is gated
by ``ui.instruct_user`` so the operator delivers the input on the device or
in a VNC client, and ``vm.metrics`` records labelled before/after snapshots
plus a 0.5 s background poll of system + per-process RSS.

Prereqs:
    - VM running with mesh-demo deployable.
    - For the runtime ``mesh::profile`` log lines, the VM's bmc-wasm-host
      must be built with ``--features profiling`` and bmc-openwrt's RUST_LOG
      must include ``mesh::profile=info``. The script does not check this —
      a missing ``mesh::profile`` log just means you only get the harness's
      own RSS timeline (still useful, just less granular).

Run:
    cd bmc-virt/harness
    just virt::harness::run examples/bdk-355-mesh-oom.py

Output:
    /tmp/bmc-virt-bdk-355/
      ├── scene-config.json
      ├── bmc-openwrt.log         (pulled from /root/bmc.log)
      ├── metrics.png             (chart with per-process RSS + tap markers)
      └── (mesh_demo.wasm copy is implicit via the build target dir)
"""

from __future__ import annotations

import json
import subprocess
from datetime import UTC, datetime, timedelta
from pathlib import Path

from bmc_virt import VM, Event, sleep, ui
from bmc_virt.paths import BMC_CONFIG, BMC_LOG

REPO_ROOT = Path(__file__).resolve().parents[3]
WASM_EXAMPLES_ROOT = REPO_ROOT / "bmc-wasm-runtime" / "examples"
WASM_MANIFEST = WASM_EXAMPLES_ROOT / "Cargo.toml"
WASM_ARTIFACT = (
    WASM_EXAMPLES_ROOT / "target" / "wasm32-unknown-unknown" / "release" / "mesh_demo.wasm"
)

OUTPUT_DIR = Path("/tmp/bmc-virt-bdk-355")
CONFIG_PATH = OUTPUT_DIR / "scene-config.json"
LOG_DST = OUTPUT_DIR / "bmc-openwrt.log"
WIDGET_LOG_DST = OUTPUT_DIR / "wasm-widget.log"
CHART_DST = OUTPUT_DIR / "metrics.png"

# Widget tracing goes to a dedicated file (widgets/wasm/src/main.rs:18) so
# it survives even when stdout/stderr inheritance to the compositor's log
# is unreliable. The mesh::profile lines we emit live here, not in BMC_LOG.
REMOTE_WIDGET_LOG = "/var/log/bmc/wasm-widget.log"

REMOTE_WASM_DIR = "/mnt/data/wasm"
REMOTE_WASM_PATH = f"{REMOTE_WASM_DIR}/mesh_demo.wasm"
WASM_WIDGET_TYPE_ID = "550e8400-e29b-41d4-a716-446655440100"

# Process names to track per-snapshot. The widget is the OOM-killer's first
# victim; the compositor is the second-stage victim when its tokio runtime
# fails to fork. Both are captured so missing-pid samples are visible in the
# chart and table.
TRACKED_PROCESSES = ["bmc-wasm-host", "bmc-openwrt"]

# Cadence of the background poller. 500 ms is fine-grained enough to see
# the per-tap upload curve without flooding the SSH channel.
POLL_INTERVAL_S = 0.5

# Order chosen to match the HW repro: D6 already seeds with Suzanne, then
# the +D20 tap is the OOM trigger on HW. Two more samples after that to
# confirm the per-die delta stays linear if the widget survives.
TAP_LADDER = ["+D20", "+D8", "+D12"]

SCENE_CONFIG = {
    "scenes": [
        {
            "id": "e430741f-f142-4f68-87b1-61280a0557c7",
            "enabled": True,
            "kind": "fullscreen",
            "widgets": [
                {
                    "id": "a6fabef7-39a0-434b-be7a-6aff41d9e711",
                    "row": 0,
                    "col": 0,
                    "size": "full",
                    "widget_type_id": WASM_WIDGET_TYPE_ID,
                    "params": {"wasmPath": REMOTE_WASM_PATH},
                }
            ],
        }
    ]
}


def build_mesh_demo() -> Path:
    ui.header("Build mesh-demo wasm")
    cmd = [
        "cargo",
        "build",
        "--manifest-path",
        str(WASM_MANIFEST),
        "-p",
        "mesh-demo",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ]

    collected: list[str] = []
    with ui.spinner("building mesh-demo..."):
        proc = subprocess.Popen(
            cmd, cwd=REPO_ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
        )
        assert proc.stdout is not None, "BUG: requested PIPE but stdout missing"
        collected.extend(line.rstrip() for line in proc.stdout if line.strip())
        rc = proc.wait()

    if rc != 0:
        ui.panel("\n".join(collected), title="cargo build failed", style="red", lexer="text")
        raise SystemExit(rc)

    if not WASM_ARTIFACT.exists():
        msg = f"Expected wasm artifact at {WASM_ARTIFACT}"
        raise SystemExit(msg)
    ui.ok(f"Built {WASM_ARTIFACT}")
    return WASM_ARTIFACT


def deploy(vm: VM, wasm: Path) -> None:
    ui.header("Deploy mesh-demo scene")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    CONFIG_PATH.write_text(json.dumps(SCENE_CONFIG, indent=2) + "\n")

    vm.exec(f'mkdir -p "{REMOTE_WASM_DIR}"')
    vm.exec(f': > "{BMC_LOG}"')
    # Widget log is append-only — truncate so we capture only this run.
    vm.exec(f': > "{REMOTE_WIDGET_LOG}"')

    vm.push(src=wasm, dst=REMOTE_WASM_PATH)
    ui.ok(f"Pushed {wasm.name} → {REMOTE_WASM_PATH}")

    vm.push(src=CONFIG_PATH, dst=BMC_CONFIG)
    ui.ok(f"Deployed config → {BMC_CONFIG}")


def restart_app(vm: VM) -> None:
    ui.header("Restart bmc-openwrt")

    # The daemon emits app.started / app.ready BEFORE the restart ack
    # arrives, so by the time we'd call wait_next() the events are already
    # in history and wait_next ignores backlog. wait_for + a timestamp
    # filter consumes them from history but rejects pre-restart hits.
    restart_started_at = datetime.now(UTC) - timedelta(seconds=1)

    ack = vm.service("b-bmc-openwrt").restart(timeout=60, verbose=True)
    if not ack.ok:
        raise SystemExit(ack.error or "b-bmc-openwrt restart failed")

    with ui.spinner("waiting for app ready..."):
        vm.wait_for(
            Event.APP_STARTED,
            timeout=60,
            where=lambda evt: evt.ts.astimezone(UTC) >= restart_started_at,
        )
        vm.wait_for(
            Event.APP_READY,
            timeout=60,
            where=lambda evt: evt.ts.astimezone(UTC) >= restart_started_at,
        )
    ui.ok("App ready")


def run_experiment(vm: VM) -> None:
    ui.header("Run BDK-355 mesh ladder")
    ui.kv("processes", ", ".join(TRACKED_PROCESSES))
    ui.kv("poll interval", f"{POLL_INTERVAL_S}s")

    with vm.metrics.start(
        "BDK-355 mesh OOM",
        interval=POLL_INTERVAL_S,
        processes=TRACKED_PROCESSES,
    ) as m:
        # Settle the seeded scene so baseline isn't mid-init transients.
        sleep(2)
        m.capture("baseline")

        # ``instruct_user`` records "<tap" on prompt and ">tap" on ack;
        # the 0.5s background poller fills the curve between, so a
        # third "settled after" snapshot would be redundant.
        for tap in TAP_LADDER:
            ui.instruct_user(tap, metrics=m)
            sleep(2)

        m.capture("final")

        m.report()
        m.chart(CHART_DST)
        ui.ok(f"Chart saved to {CHART_DST}")


def collect_logs(vm: VM) -> None:
    ui.header("Collect logs")
    vm.pull(src=BMC_LOG, dst=LOG_DST)
    ui.ok(f"Pulled {BMC_LOG} → {LOG_DST}")
    vm.pull(src=REMOTE_WIDGET_LOG, dst=WIDGET_LOG_DST)
    ui.ok(f"Pulled {REMOTE_WIDGET_LOG} → {WIDGET_LOG_DST}")
    ui.panel(
        f"mesh::profile lines (per-mesh deltas, render heartbeats):\n"
        f"  rg 'mesh::profile' {WIDGET_LOG_DST}\n"
        f"\n"
        f"Compositor frame timings:\n"
        f"  rg 'render_scene|compositor:' {LOG_DST}",
        title="Next",
        style="cyan",
        lexer="bash",
    )


def main() -> None:
    ui.header("BDK-355 — mesh OOM harness")
    ui.kv("output", str(OUTPUT_DIR))

    wasm = build_mesh_demo()

    with VM.connect(timeout=60) as vm:
        ui.ok("Connected to VM")
        vm.wait_for(Event.APP_READY, timeout=60)
        deploy(vm, wasm)
        restart_app(vm)
        run_experiment(vm)
        collect_logs(vm)


main()

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

"""Collect compositor logs for fullscreen vs 1-medium combined scene.

Deploys two configs in sequence, waits for each to settle, and pulls the
compositor log for offline analysis. No summary is computed here — run your
own grep / plotting over the saved logs.

Run against a running VM:
    cd bmc-virt/harness
    just virt::harness::run examples/combined-vs-full.py
"""

import json
from pathlib import Path

from bmc_virt import VM, Event, sleep, ui
from bmc_virt.paths import BMC_CONFIG, BMC_LOG

RECORD_SECONDS = 15
OUTPUT_DIR = Path("/tmp/bmc-virt-combined-scene")

FULLSCREEN_CONFIG = {
    "scenes": [
        {
            "id": "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d",
            "enabled": True,
            "kind": "fullscreen",
            "widgets": [
                {
                    "id": "c1a2b3c4-d5e6-4f7a-8b9c-0d1e2f3a4b5c",
                    "row": 0,
                    "col": 0,
                    "size": "full",
                    "widget_type_id": "550e8400-e29b-41d4-a716-446655440002",
                    "params": {"mode": "flat"},
                }
            ],
        }
    ]
}

SINGLE_MEDIUM_CONFIG = {
    "scenes": [
        {
            "id": "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d",
            "enabled": True,
            "kind": "combined",
            "widgets": [
                {
                    "id": "c1a2b3c4-d5e6-4f7a-8b9c-0d1e2f3a4b5c",
                    "row": 0,
                    "col": 0,
                    "size": "medium",
                    "widget_type_id": "550e8400-e29b-41d4-a716-446655440002",
                    "params": {"mode": "flat"},
                }
            ],
        }
    ]
}

SCENARIOS = [
    ("01-fullscreen", FULLSCREEN_CONFIG),
    ("02-single-medium", SINGLE_MEDIUM_CONFIG),
]


def collect_logs(vm: VM, scenario: str, scene_config: dict) -> Path:
    ui.header(f"Scenario: {scenario}")

    # Push config
    config_path = OUTPUT_DIR / f"{scenario}-config.json"
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_path.write_text(json.dumps(scene_config, indent=2))
    vm.push(src=config_path, dst=BMC_CONFIG)
    ui.ok("Config deployed")

    # Truncate the compositor log before the restart so we only capture this
    # scenario. The file is re-created with `>>` by the init script on start.
    vm.exec(f': > "{BMC_LOG}"')

    # Enable trace-level logging for compositor module
    vm.exec(
        "sed -i 's/RUST_LOG=debug/RUST_LOG=debug,bmc_openwrt::compositor=trace/' "
        "/etc/init.d/b-bmc-openwrt"
    )

    # Restart bmc-openwrt to pick up new config
    ack = vm.service("b-bmc-openwrt").restart(timeout=60, verbose=True)
    if not ack.ok:
        ui.error(f"Service restart failed: {ack.error}")
        raise SystemExit(1)
    ui.ok("Service restarted")

    # Wait for app to come up and widgets to connect
    with ui.spinner("waiting for app ready..."):
        vm.wait_next(Event.APP_STARTED, timeout=60)
        vm.wait_next(Event.APP_READY, timeout=60)
    ui.ok("App ready")

    # Let it run and accumulate log data
    ui.kv("recording", f"{RECORD_SECONDS}s")
    sleep(RECORD_SECONDS)

    # Pull compositor log — path sourced from bmc_virt.paths so it can't
    # drift from what the init script writes.
    log_path = OUTPUT_DIR / f"{scenario}-compositor.log"
    vm.pull(src=BMC_LOG, dst=log_path)
    ui.ok(f"Log saved to {log_path}")

    return log_path


def main() -> None:
    ui.header("Combined-scene log capture")
    ui.kv("output", str(OUTPUT_DIR))

    with VM.connect(timeout=60) as vm:
        ui.ok("Connected")

        vm.wait_for(Event.APP_READY, timeout=60)
        ui.ok("Initial app ready")

        for scenario, scene_config in SCENARIOS:
            collect_logs(vm, scenario, scene_config)


main()

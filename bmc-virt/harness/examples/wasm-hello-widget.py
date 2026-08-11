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

"""Render the packaged hello-widget WASM example and capture a screenshot.

The VM image ships the widget under ``lib/bmc-widgets/hello-widget/``
via the combined ``widgets-<arch>`` package, so this harness only needs
to deploy a scene config that references its widget UID.

Run against a running VM:
    cd bmc-virt/harness
    just run examples/wasm-hello-widget.py
"""

from __future__ import annotations

import json
from datetime import UTC, datetime, timedelta
from pathlib import Path

from bmc_virt import VM, Event, sleep, ui
from bmc_virt.paths import BMC_CONFIG, BMC_LOG

OUTPUT_DIR = Path("/tmp/bmc-virt-wasm-hello-widget")
SCREENSHOT_PATH = OUTPUT_DIR / "hello-widget.png"
CONFIG_PATH = OUTPUT_DIR / "scene-config.json"
WASM_LOG_PATH = OUTPUT_DIR / "wasm-widget.log"
BMC_APP_LOG_PATH = OUTPUT_DIR / "bmc-openwrt.log"

# Baked into bmc-wasm-host (see widgets/wasm/src/main.rs).
REMOTE_WASM_LOG = "/var/log/bmc/wasm-widget.log"

# UID from widgets-wasm-examples/hello-widget/manifest.json. The
# legacy generic WASM runner UID has been retired; every packaged
# widget now owns its own UID.
HELLO_WIDGET_TYPE_ID = "550e8400-e29b-41d4-a716-446655440200"
SETTLE_SECONDS = 3

SCENE_CONFIG = {
    "scenes": [
        {
            "id": "c6c34f4c-7fb5-4d10-bd41-6ea3ee512301",
            "enabled": True,
            "kind": "fullscreen",
            "widgets": [
                {
                    "id": "614ca331-a9be-4e38-bc42-f7d7735c33fb",
                    "row": 0,
                    "col": 0,
                    "size": "full",
                    "widget_type_id": HELLO_WIDGET_TYPE_ID,
                    "params": {},
                }
            ],
        }
    ]
}


def deploy_scene(vm: VM) -> None:
    ui.header("Deploy wasm scene")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    CONFIG_PATH.write_text(json.dumps(SCENE_CONFIG, indent=2) + "\n")

    vm.exec("mkdir -p /var/log/bmc")
    vm.exec(f': > "{BMC_LOG}"')
    vm.exec(f': > "{REMOTE_WASM_LOG}"')

    vm.push(src=CONFIG_PATH, dst=BMC_CONFIG)
    ui.ok(f"Deployed config to {BMC_CONFIG}")


def restart_app(vm: VM) -> None:
    ui.header("Restart bmc-openwrt")

    restart_started_at = datetime.now(UTC) - timedelta(seconds=1)
    ack = vm.service("b-bmc-openwrt").restart(timeout=60, verbose=True)
    if not ack.ok:
        raise SystemExit(ack.error or "b-bmc-openwrt restart failed")

    with ui.spinner("waiting for app restart..."):
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

    ui.ok("App restarted and reported ready")


def capture_artifacts(vm: VM) -> None:
    ui.header("Capture artifacts")

    ui.kv("settle", f"{SETTLE_SECONDS}s")
    sleep(SETTLE_SECONDS)

    screenshot = vm.screenshot(SCREENSHOT_PATH)
    ui.ok(f"Screenshot saved to {screenshot}")

    vm.pull(src=BMC_LOG, dst=BMC_APP_LOG_PATH)
    ui.ok(f"Pulled {BMC_LOG} to {BMC_APP_LOG_PATH}")

    vm.pull(src=REMOTE_WASM_LOG, dst=WASM_LOG_PATH)
    ui.ok(f"Pulled {REMOTE_WASM_LOG} to {WASM_LOG_PATH}")


def main() -> None:
    ui.header("WASM hello-widget harness")
    ui.kv("output", str(OUTPUT_DIR))

    with VM.connect(timeout=60) as vm:
        ui.ok("Connected to VM")
        vm.wait_for(Event.APP_READY, timeout=60)
        ui.ok("VM app already ready")

        deploy_scene(vm)
        restart_app(vm)
        capture_artifacts(vm)


main()

"""Build and deploy the hello-widget WASM example, then capture a screenshot.

Run against a running VM:
    cd bmc-virt/harness
    just run examples/wasm-hello-widget.py
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
HELLO_WIDGET_WORKSPACE = WASM_EXAMPLES_ROOT / "Cargo.toml"
HELLO_WIDGET_WASM = (
    WASM_EXAMPLES_ROOT / "target" / "wasm32-unknown-unknown" / "release" / "hello_widget.wasm"
)

OUTPUT_DIR = Path("/tmp/bmc-virt-wasm-hello-widget")
SCREENSHOT_PATH = OUTPUT_DIR / "hello-widget.png"
CONFIG_PATH = OUTPUT_DIR / "scene-config.json"
WASM_LOG_PATH = OUTPUT_DIR / "wasm-widget.log"
BMC_APP_LOG_PATH = OUTPUT_DIR / "bmc-openwrt.log"

REMOTE_WASM_DIR = "/mnt/data/wasm"
REMOTE_WASM_PATH = f"{REMOTE_WASM_DIR}/hello_widget.wasm"
REMOTE_WASM_LOG = "/var/log/bmc/wasm-widget.log"
WASM_WIDGET_TYPE_ID = "550e8400-e29b-41d4-a716-446655440100"
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
                    "widget_type_id": WASM_WIDGET_TYPE_ID,
                    "params": {
                        "wasmPath": REMOTE_WASM_PATH,
                    },
                }
            ],
        }
    ]
}


def build_hello_widget() -> Path:
    ui.header("Build hello-widget")

    cmd = [
        "cargo",
        "build",
        "--manifest-path",
        str(HELLO_WIDGET_WORKSPACE),
        "-p",
        "hello-widget",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ]

    collected: list[str] = []
    with ui.spinner("building hello-widget wasm...") as status:
        proc = subprocess.Popen(
            cmd,
            cwd=REPO_ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        assert proc.stdout is not None, "BUG: stdout pipe requested but missing"
        for raw in proc.stdout:
            line = raw.rstrip()
            if not line:
                continue
            collected.append(line)
            status.update(f"[dim]{line}[/dim]")
        returncode = proc.wait()

    if returncode != 0:
        ui.panel("\n".join(collected), title="cargo build", style="red", lexer="text")
        raise SystemExit(returncode)

    if not HELLO_WIDGET_WASM.exists():
        msg = f"Expected wasm artifact at {HELLO_WIDGET_WASM}"
        raise SystemExit(msg)

    ui.ok(f"Built {HELLO_WIDGET_WASM}")
    return HELLO_WIDGET_WASM


def deploy_scene(vm: VM, wasm_path: Path) -> None:
    ui.header("Deploy wasm scene")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    CONFIG_PATH.write_text(json.dumps(SCENE_CONFIG, indent=2) + "\n")

    vm.exec(f'mkdir -p "{REMOTE_WASM_DIR}" /var/log/bmc')
    vm.exec(f': > "{BMC_LOG}"')
    vm.exec(f': > "{REMOTE_WASM_LOG}"')

    vm.push(src=wasm_path, dst=REMOTE_WASM_PATH)
    ui.ok(f"Pushed {wasm_path.name} to {REMOTE_WASM_PATH}")

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

    wasm_path = build_hello_widget()

    with VM.connect(timeout=60) as vm:
        ui.ok("Connected to VM")
        vm.wait_for(Event.APP_READY, timeout=60)
        ui.ok("VM app already ready")

        deploy_scene(vm, wasm_path)
        restart_app(vm)
        capture_artifacts(vm)


main()

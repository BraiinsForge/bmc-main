"""Compare compositor callback pacing: fullscreen vs 1-medium combined scene.

Deploys two configs in sequence, collects compositor logs for each,
and prints a summary of frame-callback counts.

Run against a running VM:
    cd bmc-virt/harness
    just virt::harness::run examples/combined-vs-full.py
"""

import json
from pathlib import Path

from bmc_virt import VM, Event, sleep, ui

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
    vm.push(src=config_path, dst="/etc/bmc_config.json")
    ui.ok("Config deployed")

    # Clear old log so we only capture this scenario
    vm.exec("rm -f /tmp/bmc-openwrt.log")

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

    # Pull compositor log (see bmc-virt/rootfs/overlay/etc/init.d/b-bmc-openwrt)
    log_path = OUTPUT_DIR / f"{scenario}-compositor.log"
    vm.pull(src="/tmp/bmc-openwrt.log", dst=log_path)
    ui.ok(f"Log saved to {log_path}")

    return log_path


def summarize_log(log_path: Path) -> None:
    """Print frame-callback stats from a compositor log."""
    if not log_path.exists():
        ui.warn(f"Log not found: {log_path}")
        return

    text = log_path.read_text()
    lines = text.splitlines()
    callback_lines = [line for line in lines if "frame callbacks" in line]
    timer_lines = [line for line in lines if "headless timer tick" in line]
    timer_queued = sum(1 for line in timer_lines if "queued=true" in line)
    timer_idle = sum(1 for line in timer_lines if "queued=false" in line)

    ui.kv("frame callback log lines", str(len(callback_lines)))
    ui.kv("timer ticks (queued)", str(timer_queued))
    ui.kv("timer ticks (idle)", str(timer_idle))

    # Print last few callback lines as sample
    for line in callback_lines[-5:]:
        print(f"  {line.strip()}")


def main() -> None:
    ui.header("Combined-scene callback comparison")
    ui.kv("output", str(OUTPUT_DIR))

    with VM.connect(timeout=60) as vm:
        ui.ok("Connected")

        vm.wait_for(Event.APP_READY, timeout=60)
        ui.ok("Initial app ready")

        log_paths = {}
        for scenario, scene_config in SCENARIOS:
            log_paths[scenario] = collect_logs(vm, scenario, scene_config)

        ui.header("Results")
        for scenario, log_path in log_paths.items():
            ui.header(scenario)
            summarize_log(log_path)


main()

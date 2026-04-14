"""Showcase script — exercises the full bmc_virt harness API.

Run against a running VM:
    nix develop --command uv run python examples/showcase.py
"""

from bmc_virt import VM, Event, sleep, ui

# ── Connect to the VM ──────────────────────────────────────────────────────────

ui.header("Connect")
with VM.connect(timeout=30) as vm:
    ui.ok("Connected to event daemon")

    # ── SSH ────────────────────────────────────────────────────────────────────

    ui.header("SSH")
    result = vm.ssh("uname -a")
    ui.kv("uname", result.stdout.strip())

    result = vm.ssh("cat /etc/openwrt_release")
    ui.cmd_output(result.stdout)

    # ── Event history (backlog) ────────────────────────────────────────────────

    ui.header("Event history (backlog from VM boot)")
    for evt in vm.history:
        ui.event(evt)

    # ── Wait for events that already happened ──────────────────────────────────

    ui.header("wait_for (should return immediately from backlog)")
    evt = vm.wait_for(Event.APP_READY, timeout=5)
    ui.ok(f"app.ready at {ui.format_ts(evt.ts)}")

    evt = vm.wait_for(Event.RELAY_LISTENING, timeout=5)
    ui.ok(f"relay.listening at {ui.format_ts(evt.ts)}")

    # ── Execute shell commands via event daemon ────────────────────────────────

    ui.header("shell.exec via event daemon")
    vm.exec("echo hello from the daemon", verbose=True)
    vm.exec("ls /root/", verbose=True)
    vm.exec("free -m", verbose=True)
    vm.exec("false", verbose=True)

    # ── File transfer ──────────────────────────────────────────────────────────

    ui.header("File transfer")
    vm.pull(src="/etc/openwrt_release", dst="/tmp/bmc-virt-test-pull.txt")
    ui.ok("pulled /etc/openwrt_release → /tmp/bmc-virt-test-pull.txt")

    vm.push(src="/tmp/bmc-virt-test-pull.txt", dst="/tmp/roundtrip.txt")
    ui.ok("pushed back → /tmp/roundtrip.txt")

    ack = vm.exec("cmp /etc/openwrt_release /tmp/roundtrip.txt")
    ui.ok("roundtrip verified" if ack.ok else "roundtrip MISMATCH")

    # ── Service restart ────────────────────────────────────────────────────────

    ui.header("service.restart")
    vm.service("d-bmc-virt-relay").restart(timeout=30, verbose=True)

    ui.kv("waiting for relay to re-bind", "...")
    evt = vm.wait_next(Event.RELAY_LISTENING, timeout=30)
    ui.ok(f"relay back at {ui.format_ts(evt.ts)}")

    # ── Screenshot ──────────────────────────────────────────────────────────────

    ui.header("Screenshot")
    path = vm.screenshot("/tmp/bmc-virt-screenshot.png")
    ui.ok(f"saved to {path}")

    # ── Metrics collection ─────────────────────────────────────────────────────

    ui.header("Metrics collection")

    m = vm.metrics.start("Showcase metrics")
    m.capture("baseline")

    vm.exec("dd if=/dev/urandom of=/dev/null bs=1M count=10", timeout=10)
    m.capture("after CPU load")

    sleep(2)
    m.capture("after cooldown")

    m.report()
    chart_path = m.chart("/tmp/bmc-virt-metrics.png")
    ui.ok(f"chart saved to {chart_path}")

    # ── Stream events (brief) ──────────────────────────────────────────────────

    ui.header("Live event stream (3 seconds)")
    vm.stream(duration=3)

    ui.header("Done")

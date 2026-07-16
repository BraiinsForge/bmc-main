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

"""rr roundtrip — start recording, exercise the app, stop, pull trace.

Requires the VM to be started with --rr (deploys the rr bundle).

Run against a running VM:
    nix develop --command uv run python examples/rr.py
"""

from pathlib import Path

from rich.tree import Tree

from bmc_virt import VM, Event, sleep, ui
from bmc_virt.paths import RR_BUNDLE

ui.header("Connect")
with VM.connect(timeout=60) as vm:
    ui.ok("Connected to event daemon")

    # ── Verify the app is up before we touch rr ───────────────────────────

    ui.header("Wait for app")
    vm.wait_for(Event.APP_READY, timeout=60)
    ui.ok("app.ready")

    # ── Start rr recording ────────────────────────────────────────────────

    ui.header("rr.start")
    with ui.spinner("stopping app and starting rr..."):
        ack = vm.rr.start(timeout=30)
    if not ack.ok:
        ui.error(f"rr.start failed: {ack.error}")
        raise SystemExit(1)
    ui.ok("Recording started")

    # Wait for the app to come back up under rr
    ui.header("Wait for app under rr")
    with ui.spinner("waiting for app.started..."):
        vm.wait_next(Event.APP_STARTED, timeout=120)
    ui.ok("app.started under rr")

    with ui.spinner("waiting for app.ready (slow under rr)..."):
        vm.wait_next(Event.APP_READY, timeout=120)
    ui.ok("app.ready under rr")

    # ── Exercise the app while recording ──────────────────────────────────

    ui.header("Exercise app under rr")
    vm.exec("echo 'hello from rr'", verbose=True)
    vm.exec("ls /root/", verbose=True)
    sleep(2)

    # ── Stop recording ────────────────────────────────────────────────────

    ui.header("rr.stop")
    with ui.spinner("stopping rr and finalizing trace..."):
        ack = vm.rr.stop(timeout=30)
    if not ack.ok:
        ui.error(f"rr.stop failed: {ack.error}")
        raise SystemExit(1)

    trace = ack.data.get("trace")
    ui.ok(f"Recording stopped — trace: {trace}")

    # ── Pull trace to host ────────────────────────────────────────────────

    ui.header("Pull trace")
    local_path = Path("/tmp/bmc-virt-rr-test")
    with ui.spinner("pulling trace from VM..."):
        vm.rr.pull(local_path)
    ui.ok(f"Trace pulled to {local_path}")

    # ── Inspect the recording ─────────────────────────────────────────────

    ui.header("Trace contents")

    KIB = 1_024

    def _humanize(n: float) -> str:
        for unit in ("B", "KiB", "MiB", "GiB"):
            if n < KIB:
                return f"{n:.1f} {unit}"
            n /= KIB
        return f"{n:.1f} TiB"

    total_bytes = 0
    tree = Tree(f"[bold]{local_path.name}/[/bold]")
    for entry in sorted(local_path.rglob("*")):
        if not entry.is_file():
            continue
        rel = entry.relative_to(local_path)
        sz = entry.stat().st_size
        total_bytes += sz
        # Navigate / create intermediate tree nodes
        node = tree
        for part in rel.parent.parts:
            existing = next((c for c in node.children if c.label == f"[bold]{part}/[/bold]"), None)
            node = existing or node.add(f"[bold]{part}/[/bold]")
        node.add(f"[cyan]{rel.name}[/cyan]  [dim]{_humanize(sz)}[/dim]")

    ui.out.print(tree)
    ui.out.print()
    ui.kv("total size", _humanize(total_bytes))

    # ── Guest-side rr dump (first 30 events) ──────────────────────────────

    if trace:
        ui.header("Trace event sample (rr dump)")
        dump = vm.exec(
            f"{RR_BUNDLE}/bin/run-rr.sh dump --raw {trace} 2>&1 | head -30",
            timeout=10,
        )
        if dump.ok and dump.data.get("stdout"):
            ui.code(dump.data["stdout"])
        else:
            ui.warn("rr dump not available (app already stopped)")

    # ── Event history ─────────────────────────────────────────────────────

    ui.header("rr-related events")
    for evt in vm.history:
        if "rr." in evt.name:
            ui.event(evt)

    ui.header("Done")
    ui.panel(
        f"nix shell nixpkgs#rr -c rr replay {local_path}",
        title="Replay",
        style="green",
        lexer="bash",
    )

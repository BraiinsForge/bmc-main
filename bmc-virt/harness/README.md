# bmc-virt harness

Event-driven signaling and test harness for the bmc-virt QEMU VM.

## Goals

- **Deterministic VM lifecycle signaling** — replace sleep-based polling with event-driven waits. The guest daemon emits
  structured events (app ready, wifi connected, relay listening, etc.) and the host client consumes them over a JSONL
  TCP stream.
- **Scriptable automation** — typed Python API for test scripts: wait for events, send commands, transfer files, run
  shell commands. No interactive sessions required.
- **Shared protocol** — the same Python package runs on both host (client, CLI) and guest (daemon). Types, enums, and
  wire format are defined once.
- **Strict from day one** — ruff lint + format, ty type checking, pytest. All wired into `just validate`.

## Non-goals

- **Not a test framework** — no fixtures, decorators, or test discovery magic. A test is a Python script. Use pytest
  only for unit/integration tests of the harness itself.
- **Not a production deployment tool** — the harness is a dev-time dependency. Image size, startup time, and security
  hardening are not priorities.
- **Not a replacement for the IPC relay** — the event daemon handles signaling and commands. Framebuffer streaming stays
  on the existing binary protocol (port 5910).

## Architecture

```
Host                              Guest (OpenWrt VM)
                                  
bmc_virt.VM  ──TCP:5920──►  bmc_virt.server (eventd)
  .wait_for()                     ├─ polls: processes, ports, wifi
  .exec()                         ├─ unix socket: init script signals
  .ssh()     ──SSH:2222──►        └─ commands: shell.exec, service.restart
  .pull/push ──SCP:2222──►
```

### Wire protocol

JSONL over raw TCP, port 5920. Six message types: `hello`, `synced`, `event`, `cmd`, `ack`, `shutdown`.

Connection lifecycle:

1. Server sends `hello` (version check)
2. Server replays event backlog (all past events since boot)
3. Server sends `synced` (client is caught up)
4. Bidirectional stream: events server→client, commands client→server
5. TCP keepalive for dead peer detection (no application-level pings)

### Key choices

- **JSONL over raw TCP** instead of HTTP/SSE — bidirectional, no HTTP dependency in guest, works with `nc | jq` for
  debugging.
- **Single client model** — one consumer at a time, second connection rejected. Simplifies sequencing and avoids fan-out
  complexity.
- **Backlog replay** — new clients receive all past events on connect. `wait_for()` checks history first, returns
  immediately if event already happened. No races.
- **Python 3.11** — matches OpenWrt's `python3` package. Host devShell also uses 3.11 for consistency.
- **`from __future__ import annotations`** — deferred annotation evaluation so TYPE_CHECKING imports work without
  runtime cost. Required on 3.11 (3.14 has PEP 649 natively).
- **`rich` as sole dependency** — formatted errors, colored event streaming, CLI output. Everything else is stdlib.

## Usage

### Dev shell

```sh
cd bmc-virt/harness
nix develop    # provides python3, uv, ruff, ty
uv sync        # install deps into .venv (automatic via shellHook)
```

### Validation

```sh
just validate  # format + lint + typecheck + tests
just format    # auto-fix lint issues + format
just test      # pytest only
```

### CLI

```sh
bmc-virt ssh "uci show wireless"
bmc-virt pull /root/bmc.log ./logs/
bmc-virt push ./config.json /etc/bmc/config.json
bmc-virt events              # pretty-print live events
bmc-virt events --raw        # JSONL for piping
bmc-virt wait app.ready
bmc-virt exec shell.exec --cmd "echo hello"
```

### Python API

```python
from bmc_virt import VM, Event, Cmd, sleep

with VM.connect() as vm:
    vm.wait_for(Event.APP_READY)
    result = vm.exec(Cmd.SHELL_EXEC, cmd="echo hello")
    print(result.data["stdout"])

    vm.pull(src="/root/bmc.log", dst="./logs/bmc.log")

    # wait_next ignores backlog — waits for live occurrence only
    vm.exec(Cmd.SERVICE_RESTART, name="d-bmc-virt-relay")
    vm.wait_next(Event.RELAY_LISTENING, timeout=30)
```

## File structure

```
harness/
  bmc_virt/
    __init__.py       prelude: VM, Event, Cmd, sleep
    protocol.py       wire format (shared types, encode/decode)
    events.py         Event enum, ReceivedEvent dataclass
    commands.py       Cmd enum, Ack dataclass
    client.py         JSONL TCP client (host-side)
    server.py         event daemon (guest-side)
    vm.py             high-level VM handle
    ssh.py            SSH/SCP via subprocess
    cli.py            CLI entry points
    rr.py             rr debugger API (stub)
    _print.py         rich output helpers (stub)
  tests/
    test_protocol.py        unit tests (no VM)
    test_client_server.py   integration tests (loopback TCP, no VM)
  examples/
    showcase.py             full API demo against a running VM
```

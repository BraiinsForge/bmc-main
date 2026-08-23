# Kill the Remaining `DECK_*` Env Vars

## Context

The earlier BDK-386 work moved *settings* (timezone, night mode, localization) from env vars to typed `deck_widget`
events. It left five env vars on every widget spawn — the pieces of state the widget needed synchronously at `main()`
entry, before it could do the first Wayland roundtrip:

```
DECK_INSTANCE_ID    # UUID echoed back in get_widget_surface(instance_id)
DECK_SIZE_TYPE      # small | medium | large | full
DECK_WIDTH          # pixel width
DECK_HEIGHT         # pixel height
DECK_PARAMS         # per-widget config as a JSON blob
```

This devlog records the subsequent removal of these variables.

## End state

A widget's environment on the device is now, verbatim:

```
WAYLAND_DISPLAY=wayland-0
XDG_RUNTIME_DIR=/tmp/runtime-xxxx
```

Verified on a Deck with `cat /proc/$(pgrep bmc-widget-flip-clock)/environ`. Zero `DECK_*` left.

Every piece of per-widget state is delivered via typed `deck_widget` events emitted on `get_widget_surface`, terminated
by a new `configure_done` sentinel.

## Protocol shape

### Removed

- `get_widget_surface(surface, instance_id)` — the `instance_id` arg. The compositor now resolves identity from the
  Wayland socket's peer credentials (`SO_PEERCRED`), removing the only reason the widget needed to echo the UUID back.
- Error enum entry `invalid_instance`.

### Added on `deck_widget_surface_v1`

```xml
<event name="configure">
  <arg name="size_type" type="uint" enum="size_type"/>
  <arg name="width"     type="uint"/>
  <arg name="height"    type="uint"/>
</event>
<event name="params">
  <arg name="json" type="string" summary="JSON-encoded params object"/>
</event>
<event name="configure_done"/>
```

Plus a `size_type` enum that mirrors `bmc_widget_protocol::SizeType`.

### Event ordering contract

On `get_widget_surface` the compositor emits, in order:

1. `configure(size_type, width, height)` — exactly once.
2. `params(json)` — the widget's per-instance params as a JSON-encoded object, exactly as stored in the scene config.
3. Current `timezone` / `night_mode` / `date_format` / … setting events.
4. `configure_done` — sentinel the widget blocks on before first render.

After `configure_done`, setting events continue to flow on change.

## Architecture

Before:

```
coordinator ──RegisterWidget(iid, pos, size, None)──> compositor
coordinator ──env={DECK_*}── spawn(widget) ──> widget process
                                                  │
widget main():                                    │
  iid  = env::var(DECK_INSTANCE_ID)               │
  size = env::var(DECK_SIZE_TYPE)                 │
  w,h  = env::var(DECK_WIDTH/HEIGHT)              │
  params = json::from_str(env::var(DECK_PARAMS))  │
  build renderer with known config                │
  wayland::connect()                              │
  get_widget_surface(surface, iid) ──────────────>│── string compare iid → widget
```

After:

```
coordinator ──RegisterWidget(iid, pos, size, initial_config) ack────>┐
coordinator ──env={WAYLAND_DISPLAY,XDG_RUNTIME_DIR}── spawn ──> widget
coordinator ──SetWidgetPid(iid, pid) ack─────────────────────────────>│
                                                                      │
widget main():                                                        │
  wayland::connect()                            SO_PEERCRED(sock)─────│── pid → iid
  (surface, initial) = DeckWidgetSurfaceClient  emit configure        │
    ::connect()  ── blocks on configure_done    emit params(json)     │
  build renderer from initial.{size,w,h,params} emit timezone/…       │
  run_loop(initial.settings)                    emit configure_done ──┘
```

The substantive shift: configuration lives in the compositor's protocol state, not the widget's environment. Identity is
a kernel-enforced property of the socket, not a UUID the widget could lie about.

## Key implementation decisions

### 1. SO_PEERCRED, not a per-instance socket

Three options were considered. We picked `SO_PEERCRED` because smithay's existing accept loop already returns a plain
`UnixStream`, so `client.get_credentials(dhandle)` drops into the existing code path `instance_id_for_surface_by_pid`
was already using for Slint render surfaces. Zero smithay surgery.

The PID→instance map lives in `DeckWidgetProtocolState.widgets` (a `HashMap<InstanceId, WidgetData>` with a `pid`
field), populated by the new `SetWidgetPid` command.

### 2. Synchronous registration, not a pending-connection buffer

Races were the biggest concern: coordinator sends `RegisterWidget`, spawns the process, the process starts a Wayland
roundtrip, compositor peer-credentials the connection — all of this happens in milliseconds and the order matters. A
pending buffer that retries identity lookup at `get_widget_surface` time would work but adds a state machine.

Simpler: make `register_widget()` and `set_widget_pid()` on the `Compositor` trait *synchronous* via a flume ack
channel. The coordinator blocks at most 2s (in practice microseconds) waiting for the compositor to store the record
before it returns, and won't spawn until registration is durable. After spawn it blocks again waiting for
`set_widget_pid` to land.

```rust
CompositorCommand::RegisterWidget {
    instance_id, position, size, initial_config,
    ack: flume::Sender<()>,
}
```

### 3. Params on the wire: a single JSON event, widget owns the schema

Params are heterogeneous per widget — flip-clock has `{mode: string}`, digital-clock has
`{showSeconds: bool, fontStyle: string, timezone: ?str}`. The protocol can't know the keys. Options were:

- One event carrying the whole JSON object.
- One event per param type (`param_string`, `param_bool`, `param_number`, `param_null`) — initially implemented on the
  wire, but that meant every numeric precision quirk (Wayland's `fixed` 24.8, re-decoded as `f64`) and every new param
  shape needed a protocol round, for data the widget's manifest already describes.
- One event per key known to any manifest — combinatorial explosion.

Shipped: a single `params(json)` event carrying the widget's configured params object verbatim. The compositor doesn't
parse or validate the payload; the widget's manifest is authoritative over its own params. Reversing the typed-per-value
wire shape was deliberate — the initial typed events (commit `35ab9921`) introduced two versioning surfaces (Wayland
enum of types + per-key JSON schema) for the same data; the single event restores "one channel, one schema."

### 4. Initial state as a batch, with `configure_done` as a sentinel

Without an explicit "batch complete" signal the widget can't tell a quiet compositor from a crashed one.
`configure_done` mirrors `xdg_surface.configure` in spirit. `DeckWidgetSurfaceClient::connect()` and
`WidgetProtocolClient::wait_for_configure()` both block on it with a 10-second timeout — past that, the widget exits
with an `anyhow::Error` and the spawner restarts it.

## What changed where

**Protocol** (`bmc-widget-protocol`):

- `protocol/deck-widget.xml` — new events + enum, removed `instance_id` arg and `invalid_instance` error.
- `src/types.rs` — new `WidgetInitialConfig { size, width, height, params }`.

**Compositor** (`bmc-openwrt/src/compositor/`):

- `protocol/state.rs` — `initial_configs: HashMap<InstanceId, …>`, `current_settings` storage (moved from `AppState`),
  `register_initial_config`, `set_widget_pid`, `attach_surface`, `emit_initial_state`.
- `protocol/dispatch.rs` — `GetWidgetSurface` resolves identity via `peer_cred()`, drops widgets whose pid isn't
  registered, emits the initial batch.
- `protocol/conversions.rs` — `size_type_to_protocol`.
- `commands.rs` — `RegisterWidget` carries `WidgetInitialConfig` and an `ack` channel; new `SetWidgetPid` variant.
- `egl_compositor.rs` — synchronous command handlers; `AppState` shrunk (no more `current_settings` duplication).

**Coordinator** (`bmc/src/`):

- `compositor.rs` — `Compositor` trait: `register_widget` signature changed, `set_widget_pid` added.
- `widget/coordinator.rs` — register + spawn + set_widget_pid sequence; `WidgetEnv` shrunk to
  `{instance_id, wayland_display}`.
- `widget/spawner.rs` — `env_clear()`; only `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, plus `PATH` and `RUST_LOG` if set in
  the parent.
- `widget/manager.rs` — `spawn_widget()` now returns `u32` pid.

**Widget client library** (`bmc-widget/src/`):

- `env.rs` — **deleted entirely** (192 LOC gone).
- `params.rs` — new module: `WidgetParams`, `ParamValue`, `ParamError` with typed accessors (`get_string`,
  `get_bool_opt`, …).
- `surface/deck_widget.rs` — `connect()` no longer takes width/height; returns `(Self, InitialState)` after blocking for
  `configure_done`.
- `wayland.rs` — `WidgetProtocolClient::create_widget_surface()` no longer takes `instance_id`; new
  `wait_for_configure()` for Slint-based widgets that share the protocol connection only for settings.

**Widget binaries**:

- `widgets/flip-clock/src/{main.rs, ipc.rs, wayland.rs}` — invert the flow: connect, wait for configure, decode `mode`
  from params, build renderer, run loop.
- `widgets/digital-clock/src/{main.rs, ipc.rs, params.rs, lib.rs}` — same inversion; removed obsolete
  `Params`/`ParamFontStyle` serde types in favor of protocol-side decoding.

Net: ~1k insertions, ~600 deletions across 26 files.

## Verification

Built clean for ARMv7 via `.#armv7-glibc-release`:

- `target/armv7-unknown-linux-gnueabihf/release/bmc-openwrt` (13 MB)
- `target/armv7-unknown-linux-gnueabihf/release/bmc-widget-flip-clock` (4.4 MB)
- `target/armv7-unknown-linux-gnueabihf/release/bmc-widget-digital-clock` (2.7 MB)

Deployed to a Deck via `scripts/nix-cargo-deploy.sh`.

Environment verification — `cat /proc/$(pgrep bmc-widget-flip-clock)/environ`:

```
WAYLAND_DISPLAY=wayland-0
XDG_RUNTIME_DIR=/tmp/runtime-xxxx
```

Zero `DECK_*` remained.

The extruded→flat visual test was run (config flip + compositor restart) — the flip-clock re-rendered with the new
animation mode, confirming params flow through the protocol end to end.

`bmc-widget-protocol` and `bmc-widget` unit tests pass (22 tests total).

## Not in scope (deferred)

The following remained deliberately deferred:

- **`config.json` format** unchanged.
- **Hot-reload of params without widget restart** — coordinator still respawns the process on param change.
- **`bmc-ipc`'s parallel `ActionPayload`** — legacy IPC module still coexists. Separate cleanup.
- **Protocol versioning** — still at `version="1"`. Bump when we add a new event in a non-breaking way.
- **New param primitive types** (colors, arrays, …) — current four cover every manifest param in use.

## References

- Previous step: `docs/devlogs/BDK-386/wayland-env-consolidation.md`
- Ticket: BDK-386

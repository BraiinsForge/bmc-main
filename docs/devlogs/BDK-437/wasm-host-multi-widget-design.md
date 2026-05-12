# BDK-437 — WASM host runtime: N widgets per host process

## Motivation

The Braiins Deck has 250 MB of RAM, of which 132 MB is reserved for CMA (GPU/DMA buffers). Each WASM widget today runs
as a standalone OS process: it creates its own EGL context, its own `glow::Context`, its own femtovg `Renderer`, its own
Tokio reactor, and its own DMA-BUF export ring. EGL initialization plus the first render allocate roughly 7 MB of RSS
per process. With N widgets, the cost scales linearly and consumes a large fraction of available memory.

This document specifies a refactor where one host runtime process can host N WASM widgets simultaneously, sharing every
heavy resource (EGL context, renderer, font cache, Tokio reactor, HTTP/mDNS clients) while preserving the existing
per-widget OS-process identity that the compositor and the BMC coordinator depend on.

A secondary goal: framebuffers (DMA-BUF export rings, staging FBOs) are pooled across widgets and held only while a
widget is on-screen or about to be. A new lifecycle protocol on `deck-widget-v1` drives the pool.

## Non-goals

- Cross-widget GL resource sharing beyond what falls out of a shared context (no texture atlasing, no shared shader
  programs beyond the blit).
- Dropping wasmi `Store` state for cold widgets. Dormant widgets keep their WASM state warm; only render targets are
  pooled.
- Replacing the wasmi engine, the femtovg renderer, the Wayland protocol, or any compositor scene logic.
- Multi-host topologies. The design permits more than one host (versioned socket paths support parallel installs) but is
  optimized for a single global host.

## Architecture

```
                                  ┌──────────────────────────────┐
   coordinator (bmc/widget)       │  bmc-wasm-host (daemon)      │
        │                         │                              │
        │ spawn(bmc-wasm-thin     │  - one EGL context           │
        │       --wasm X.wasm     │  - one Renderer / font cache │
        │       --params JSON)    │  - one Tokio reactor         │
        ▼                         │  - shared blit shader        │
   ┌────────────┐                 │  - framebuffer pool          │
   │ bmc-wasm-  │  ctrl socket    │  - N WidgetSlot              │
   │   thin     │ ──SCM_RIGHTS──► │      ├ wasmi Store           │
   │ (PID-X)    │  (wayland fd +  │      ├ wl_display + surface  │
   │            │   wasm path +   │      ├ render target (opt)   │
   │ idle       │   params + ID)  │      └ per-widget I/O regs   │
   └────────────┘                 │                              │
        │                         │  listens on                  │
        │ also opens              │   /run/bmc/wasm-host-v{N}.sock│
        ▼ Wayland connection      └──────────────────────────────┘
   ┌────────────┐                            ▲
   │ compositor │ ←──── wayland fd ──────────┘ SO_PEERCRED still
   │ deck_widget│                              returns thin's PID
   │   _v1      │
   └────────────┘
```

The thin wrapper exists per widget instance, holds the OS-visible PID, opens the Wayland connection (so the compositor's
`SO_PEERCRED`-based identity is unchanged), passes the connected Wayland fd to the host via `SCM_RIGHTS`, then idles on
a control socket. When the coordinator kills the thin process, its control socket closes and the host drops that
widget's slot.

The compositor is unaware of the host runtime; from its perspective, each widget still arrives as a single Wayland
client identified by a unique PID.

## Crate structure

| Crate                 | Role                                                                                                                                                                                        | Lifetime                                                                                           |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `bmc-wasm-host` (new) | Daemon binary. Owns EGL context, `Renderer`, Tokio reactor, slot table, framebuffer pool. Listens on `/run/bmc/wasm-host-v{major}.sock`.                                                    | One process per host major version, started lazily by the first thin wrapper that finds no socket. |
| `bmc-wasm-thin` (new) | Thin wrapper binary. Connects to Wayland, sends fd + load command to host, idles as lifetime witness.                                                                                       | One process per widget instance, spawned by the BMC coordinator.                                   |
| `bmc-wasm-runtime`    | Refactored: `HostState` split into per-widget fields plus an `Rc<SharedHost>`; `WasmWidgetRuntime::new` no longer constructs the `Renderer`. Network/Tokio singletons move to `SharedHost`. | Library used by `bmc-wasm-host`.                                                                   |
| `bmc-widget`          | `egl::EglState` split into `EglContext` (singleton) plus `WidgetExportRing` (per surface). Existing native widgets keep using a thin owns-both wrapper that preserves today's 1:1 behavior. | Library.                                                                                           |
| `widgets/wasm/`       | Removed; replaced by `bmc-wasm-thin`.                                                                                                                                                       | —                                                                                                  |

Socket path uses major version only (`v{major}`), since minor/patch are backwards-compatible within a major.

## Lifecycle

### Thin wrapper startup

1. Parse `--wasm <path>` and forwarded params.
2. Open a Wayland connection (`$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` per convention) at the socket layer only — do not bind
   any globals or send any protocol traffic. Keep the raw fd. Peer credentials on the socket are latched by the kernel
   at `connect()` time and reflect the thin's PID/UID/GID; when the compositor later samples `SO_PEERCRED` on the server
   end (during `get_widget_surface` dispatch, as today), it sees the thin process even after the fd has been passed to
   the host.
3. Try to connect to `socket_path(major)`. On success, jump to step 6.
4. On `ECONNREFUSED` / `ENOENT`: open `lockfile_path(major)` with `O_CREAT | O_RDWR`, take `flock(LOCK_EX)`. Re-attempt
   step 3. If still failing, `fork()`+`exec("bmc-wasm-host")`. Poll the socket with bounded timeout (~2 s). On timeout,
   exit non-zero.
5. Drop the flock.
6. Send `Hello { wasm_path }` on the control socket, passing the Wayland fd via `SCM_RIGHTS` in the same `sendmsg`. The
   widget identity travels with the Wayland connection (set by the compositor via `SO_PEERCRED`), so the thin wrapper
   does not carry it on the control socket; `params` and settings arrive on the host's side via the Wayland configure
   batch.
7. Read `Ack`. On `Err(msg)`, log and exit non-zero. On `Ok`, idle.
8. Block reading the control socket. On EOF / EPIPE / SIGTERM, exit cleanly.

The thin wrapper accepts `--host-socket <path>` to override the canonical path. Useful for tests and `bmc-mock`.

### Host startup (lazy)

1. Bind the socket. On `EADDRINUSE` of a stale socket, unlink + retry once. On a fresh `EADDRINUSE`, exit (another host
   won the spawn race).
2. Initialize `EglContext`, `glow::Context`, `Renderer`, blit shader, font cache.
3. Start the Tokio runtime on a dedicated worker thread.
4. Enter the main loop (§ Render orchestration).

The host stays alive after the last widget disconnects for a bit (like 100 ms) and then quits as well. The first widget
spawning will spawn it again if necessary.

### Per-widget setup (host side)

1. `accept(4)` on the control socket → new thin connection.
2. `recvmsg` with `SCM_RIGHTS` → `Hello` frame + Wayland fd.
3. Wrap the fd in a `wayland_client::Connection`. Drive the full handshake: `wl_display.get_registry`, bind
   `deck_widget_manager_v1`, call `get_widget_surface`, wait for `configure_done`. The compositor's `SO_PEERCRED` lookup
   on this connection authoritatively binds the resulting surface to the widget instance the coordinator registered for
   the thin's PID. The host captures the initial size, params JSON, and settings from the configure batch.
4. Load WASM from `wasm_path`, validate `__bmc_sdk_version` major-equality, construct `WasmWidgetRuntime` against the
   shared `glow::Context`.
5. Send `Ack::Ok` on the control socket. Insert the slot in the table with initial lifecycle state `dormant`.

### Per-widget teardown

Triggered by any of:

- Control socket EPIPE (thin process exited).
- Wayland disconnect for this widget.
- `runtime.render()` returning `RenderStatus::Dead`.

Teardown: abort all per-widget Tokio tasks (fetches, websockets, sockets, mDNS browses, SSDP, UDP, HTTP listeners); drop
the `WasmWidgetRuntime`; release the render target (if any) to the pool; close the per-widget Wayland fd; close the
control socket if still open; remove the slot from the table.

## Control protocol (thin ↔ host)

SOCK_STREAM, length-prefixed binary frames. Protocol is versioned by the major in the socket path, so no in-band
negotiation.

```rust
// thin → host, sent once immediately after connect()
struct Hello {
    wasm_path: String,
    // Wayland fd carried out-of-band via SCM_RIGHTS
}

// host → thin, sent once in response
enum Ack {
    Ok,
    Err(String),
}
```

After `Ack::Ok` no further messages flow. The channel exists purely as a lifetime witness; EOF / EPIPE means "thin
gone".

`wasm_path` is a path, not the module bytes, so the thin wrapper's RSS stays minimal. The host opens the file directly.

## Render orchestration

### Resource ownership

| Resource                                                                           | Owner            | Notes                                                                                                |
| ---------------------------------------------------------------------------------- | ---------------- | ---------------------------------------------------------------------------------------------------- |
| `EglContext`, `glow::Context`                                                      | Host (singleton) | One context made current at host start, never switched.                                              |
| `Renderer` (femtovg `Canvas`, fonts, paths)                                        | Host (singleton) | Accessed by host functions during `slot.render()` via `*mut Renderer` set on the slot's `HostState`. |
| Blit shader, VBO, blit `Program`                                                   | Host (singleton) | Reused for every widget's Y-flip blit.                                                               |
| Tokio runtime, `reqwest::Client`, mDNS daemon, SSDP listener, UDP broadcast socket | Host (singleton) | Per-widget isolation via per-widget handle maps in `HostState`.                                      |
| `WasmWidgetRuntime` (wasmi `Store`, linear memory)                                 | Per-widget slot  | Lives for the slot's full lifetime, including `dormant`.                                             |
| `wl_display`, `wl_surface`, event queue                                            | Per-widget slot  | One connection per widget.                                                                           |
| Export ring (DMA-BUFs), staging FBO + stencil RBO                                  | Pool             | Acquired on `prepared`+ transitions, released on `dormant`.                                          |

### Renderer access from host functions

All renderer access today is bracketed by `runtime.renderer().begin_frame(…)` and `runtime.renderer().flush()` around
`runtime.render(delta_ms)`. Host import modules other than `render.rs` do not touch the renderer. Async I/O delivery
(`deliver_fetch_responses`, `deliver_ws_messages`, …) does not touch the renderer.

The host therefore passes the renderer by raw pointer into `HostState` for the duration of one render call, via a guard:

```rust
impl WidgetSlot {
    fn render(&mut self, renderer: &mut Renderer, delta_ms: u32) -> Result<RenderStatus> {
        renderer.begin_frame(self.width, self.height, 1.0);
        let status = self.runtime.with_renderer(renderer, |rt| rt.render(delta_ms))?;
        renderer.flush();
        Ok(status)
    }
}
```

`with_renderer` sets the pointer in `Store::data_mut()`, runs the closure, and clears the pointer on both normal exit
and panic (via `Drop`). Host functions read the pointer or panic with
`expect("BUG: renderer accessed outside render scope")`.

### Main loop

```rust
loop {
    let timeout = compute_poll_timeout(&slots);
    poll(&mut all_fds, timeout);

    accept_new_thin_connections()?;
    for slot in slots.values_mut() {
        slot.dispatch_wayland_events()?;
        slot.dispatch_control_events()?;
    }

    for slot in slots.values_mut() {
        slot.deliver_async_io(&shared);
    }

    for slot in slots.values_mut().filter(|s| s.needs_render()) {
        slot.render(&mut renderer, &mut egl)?;
    }
}
```

`needs_render()` is true when the slot's lifecycle state is in `{prepared, entering, visible, leaving}` AND the slot is
dirty (either a fresh frame is required by the state machine, or `runtime.wants_next_frame()` returned true).

Rendering is strictly serialized: the EGL context is current to the host's single context; each iteration only changes
which FBO is bound. There is no `eglMakeCurrent` thrash and no thread synchronization.

### Poll timeout

`compute_poll_timeout` returns the minimum of:

- `runtime.next_frame_delay()` for every `visible` / `leaving` slot.
- 100 ms if any slot has pending async I/O (today's behavior).
- 0 if any slot is already dirty.
- `-1` (infinite) otherwise.

## Widget lifecycle protocol

Added to `bmc-widget-protocol/protocol/deck-widget-v1.xml`:

```xml
<enum name="lifecycle_state">
  <entry name="dormant"  value="0"/>
  <entry name="prepared" value="1"/>
  <entry name="entering" value="2"/>
  <entry name="visible"  value="3"/>
  <entry name="leaving"  value="4"/>
</enum>

<event name="lifecycle">
  <description summary="widget lifecycle state changed">
    Emitted whenever the widget's lifecycle state changes. The widget
    starts in 'dormant' immediately after the initial configure_done
    batch. Transitions occur driven by the compositor's scene logic
    (scene cycle position, active drag, neighbor relationship).
  </description>
  <arg name="state" type="uint" enum="lifecycle_state"/>
</event>
```

### States

- **dormant** — far from active scene; no render target; runtime not ticked.
- **prepared** — neighbor of active scene; render target allocated; one pre-render frame committed; no animation tick.
- **entering** — on-screen transition in progress; re-render once.
- **visible** — active on-screen; full render loop with animation and frame callbacks.
- **leaving** — on-screen transition out; continue rendering until the transition completes.

| State    | wasmi Store | Async I/O | Render target | Render loop    | Frame cb |
| -------- | ----------- | --------- | ------------- | -------------- | -------- |
| dormant  | warm        | delivered | none          | paused         | —        |
| prepared | warm        | delivered | allocated     | one pre-render | —        |
| entering | warm        | delivered | allocated     | re-render once | —        |
| visible  | warm        | delivered | allocated     | full loop      | yes      |
| leaving  | warm        | delivered | allocated     | full loop      | yes      |

### Transitions

| From               | To       | Host action                                           |
| ------------------ | -------- | ----------------------------------------------------- |
| dormant            | prepared | pool-acquire ring + staging; render one frame; commit |
| prepared           | entering | re-render once                                        |
| entering           | visible  | enable animation tick                                 |
| visible            | leaving  | continue full loop                                    |
| leaving            | prepared | keep render target; idle                              |
| leaving / prepared | dormant  | release render target to pool                         |
| dormant            | visible  | (abrupt) acquire ring; full render; enable animation  |

The state machine is total (every (state, event) pair has a defined transition) and tolerates arbitrary jumps emitted by
the compositor.

### Compositor source of truth

`bmc-openwrt/src/compositor/widget_tracker.rs` maps current scene state to lifecycle states:

- Active widget = `visible`.
- Immediate neighbor in the scene cycle = `prepared`.
- All others = `dormant`.
- During an active drag: outgoing = `leaving`, incoming = `entering`. On drag settle, snap to `visible`/`prepared`.

## Framebuffer pool

The host owns a pool of `WidgetExportRing` + `StagingBuffer` instances, sized for the maximum simultaneous on-screen
widget count (1–8, with 8 as the hard ceiling). Pool entries are allocated lazily on first need and reclaimed under
memory pressure or after an idle timeout.

`Pool::acquire(width, height)` returns an entry whose dimensions match the request. If no matching entry is free, the
pool grows up to a configured ceiling. Beyond the ceiling, `acquire` returns an error; the slot stays in its old state,
the visibility transition is logged as failed, and no commit is sent. The ceiling is chosen empirically based on
observed worst-case visible counts.

`Pool::release(entry)` returns the entry to the free list; the host waits for any in-flight buffer to be returned by the
compositor (existing `wl_buffer.release` handling) before reusing it.

All widgets currently target the same display resolution, so pool entries are interchangeable. If a future widget needs
a different size, the pool keys on `(width, height)`.

### Memory-pressure behavior at pool ceiling

The system cannot allocate past the ceiling. The goal is deterministic degradation, not best-effort guessing.

- **Admission priority:** `visible` > `entering` > `prepared`. Under pressure, `prepared` work is dropped first.
- **Reserve for active path:** keep at least one ring budgeted for the active `visible` widget path.
- **Reclaim order:** reclaim free entries oldest-first; prefer reclaiming sizes with no currently `visible` users.
- **Idle reclaim:** free unused pool entries after a timeout to avoid pinning peak CMA indefinitely.
- **Pressure reclaim:** when CMA free memory falls below a watermark, trigger immediate reclaim of idle/free entries.

If `Pool::acquire` still fails after reclaim:

- **`dormant -> prepared` failure:** keep slot `dormant`; do not emit partial transition commits.
- **`dormant -> visible` or `prepared -> entering` failure:** keep last committed buffer if one exists, otherwise remain
  blank; mark slot as `resource_blocked`.
- **`visible` with existing target:** continue rendering on current target; no forced drop.
- **Retry policy:** retry blocked slots on next release event and on a bounded timer.

Observability:

- emit rate-limited warning logs with widget identity, requested size, pool usage, and lifecycle state;
- expose counters for `acquire_fail`, `reclaim_runs`, `blocked_slots`, and `blocked_duration_ms`;
- emit a host->compositor/coordinator pressure signal so scene logic can prefer lower-memory transitions while pressure
  is active.

## Host API isolation

```rust
pub struct HostState {
    // per-widget, private
    fuel_budget: FuelBudget,
    rng: XorShift32,
    fetch_handles: HashMap<FetchId, JoinHandle<FetchResult>>,
    websockets:    HashMap<WsId,    WsHandle>,
    sockets:       HashMap<SockId,  SocketHandle>,
    mdns_browses:  HashMap<MdnsId,  MdnsHandle>,
    ssdp_searches: HashMap<SsdpId,  SsdpHandle>,
    udp_broadcasts:HashMap<UdpId,   UdpHandle>,
    http_listeners:HashMap<HttpId,  HttpHandle>,
    audio_plays:   HashMap<AudioId, AudioHandle>,
    led_effects:   HashMap<LedId,   LedHandle>,
    settings:      WidgetSettings,
    last_render_target_size: (u32, u32),
    renderer_ptr:  Option<NonNull<Renderer>>,
    /// Sender for this widget's own Wayland surface — `host_play_sound`,
    /// `host_led_temporary`, etc. dispatch into this channel, the host's
    /// main loop drains it and emits the corresponding requests on this
    /// widget's `deck_widget_surface_v1`. The compositor disambiguates by
    /// the connection the request arrives on.
    wayland_out: WaylandSurfaceTx,

    // shared, held by the host
    shared: Rc<SharedHost>,
}

pub struct SharedHost {
    glow: glow::Context,
    blit: BlitShader,
    reqwest: reqwest::Client,
    tokio: tokio::runtime::Handle,
    mdns: Arc<MdnsDaemon>,
    system_clock_offset: AtomicI64,
}
```

The renderer itself is owned outside any `HostState` (on the host's stack during `slot.render()`) and reached only
through `renderer_ptr`. Tokio JoinHandles registered in per-widget maps are aborted on slot drop, freeing all per-widget
resources.

`Rc` instead of `Arc`: the main loop is single-threaded. The Tokio reactor runs on a worker thread, but the wasmi
`Store` and `HostState` never leave the main thread.

System time uses a global offset (`AtomicI64`). RNG state is per-widget to prevent correlated streams.

### Widget identity

The host runtime does not maintain its own widget-identity table. The compositor authoritatively binds each widget
instance to a Wayland connection (via `SO_PEERCRED` at `connect()` time). The host learns the widget's properties from
the initial Wayland configure batch (size, params JSON, settings). Audio, LED, sound, and any other commands that BMC
tags per-widget are emitted as requests on the widget's own `deck_widget_surface_v1`; the compositor identifies the
widget from the Wayland connection and forwards to BMC with the correct identity. The host neither knows nor needs to
know the numeric `instance_id` for routing.

If a future feature requires the host to log or correlate against `instance_id`, the protocol can be extended with an
`instance_id(u128)` event delivered as part of the configure batch.

## Isolation

One process now hosts N widgets. The design must guarantee that one widget cannot corrupt, observe, starve, or crash
another. Isolation is provided by a combination of wasmi's sandbox, per-widget data structures, and explicit caps.

### The five rules for per-widget async I/O

Every async I/O facility exposed to widgets (HTTP fetch, WebSocket, raw socket, mDNS browse, SSDP search, UDP broadcast,
HTTP listener, audio play, LED effect) follows the same five rules. Implementations that deviate are considered bugs.

1. **Per-widget ID namespace.** Each resource class has its own `HashMap<Id, Handle>` field on `HostState`. IDs are
   allocated from a per-widget counter. `WsId=5` in widget A and widget B are unrelated map keys in unrelated maps.
   There is no global ID registry.
2. **Per-widget delivery channel captured at spawn.** Each background task is spawned with a clone of a
   `tokio::sync::mpsc::Sender` field on the owning widget's `HostState`. The sender is the **only** route into that
   widget. There is no global "look up widget by ID and deliver" path.
3. **No connection or engine state shared between widgets.** Widget A and widget B opening connections to the same URL
   get independent TCP sockets, TLS sessions, in-flight request state. Shared engines (`reqwest::Client`, mDNS daemon)
   are stateless with respect to caller identity — they multiplex transport, not session.
4. **Per-widget concurrency cap.** Each resource class has a cap on simultaneously-open handles. `host_ws_connect` (and
   equivalents) return an error when the cap is reached. Caps bound fd consumption, memory, reactor task count.
5. **All per-widget tasks aborted on slot drop.** Slot teardown iterates every per-widget map and calls
   `JoinHandle::abort()` on each handle. No background task outlives its owning widget.

Concrete initial caps (tunable in implementation):

| Resource                        | Cap per widget |
| ------------------------------- | -------------- |
| HTTP fetches in flight          | 8              |
| WebSockets open                 | 4              |
| Raw sockets open                | 8              |
| mDNS browses                    | 4              |
| SSDP searches                   | 2              |
| UDP broadcast sockets           | 2              |
| HTTP listeners                  | 1              |
| Audio plays in flight           | 4              |
| LED effects active              | 4              |
| Inbound queue depth per channel | 256            |

### wasmi-enforced isolation

| Concern                    | Mechanism                                                                                                        |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Linear memory access       | Separate `Store` per widget; wasmi blocks cross-store access by construction.                                    |
| Linear memory growth       | `StoreLimitsBuilder` cap (initial value: 16 MB).                                                                 |
| CPU consumption per render | Existing per-render fuel budget; `RenderStatus::Dead` after repeated overages.                                   |
| Host fn dispatch routing   | `Caller<HostState>` is bound to the calling `Store`; `caller.data_mut()` is statically the right widget's state. |

### Renderer / GL state hygiene

The renderer is shared, so each widget's `slot.render()` must start from a known state and not leak state to the next
widget:

- The slot binds its own staging FBO and viewport before calling into `runtime.render()`.
- The slot calls `renderer.begin_frame(w, h, 1.0)`, which resets femtovg transforms, paths, clips, and scissor.
- After `runtime.render()`, `renderer.flush()` drains pending draws.
- The blit pass explicitly sets its own program, VBO, and uniforms.
- GL caps (depth test, stencil test, scissor) are normalized at the start of `slot.render()` so prior state cannot
  affect the new frame.

A unit test verifies that two sequential `begin_frame` + arbitrary draws + `begin_frame` cycles produce identical output
for the second frame's draws — proving no femtovg-internal bleed.

### Panic safety

A panic inside a host function (e.g., a guest passing pathologically bad arguments that hit an `unwrap` in untrusted
code paths) must not kill the host. `slot.render()` is wrapped in `std::panic::catch_unwind`:

```rust
let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
    slot.render(&mut renderer, delta_ms)
}));
match result {
    Ok(Ok(status)) => { /* normal */ }
    Ok(Err(_)) | Err(_) => {
        // treat as RenderStatus::Dead — drop the slot
    }
}
```

Background tasks (per-widget Tokio tasks) are already panic-isolated: `JoinHandle` resolves with a `JoinError` of kind
"panicked", which the deliver path converts into a normal "resource dead, close ID" event for that widget alone.

### Compositor side

Each widget has its own Wayland connection, so:

- Wayland protocol object IDs live in disjoint object-ID spaces — widget A cannot reference widget B's `wl_surface`,
  buffer, or callback.
- A buffer attached to widget A's surface is reclaimed only on its own `wl_buffer.release` event; no cross-surface
  scheduling effects.

### Audio / LED

Audio (`play_sound`, `stop_sound`) and LED (`led_temporary`, `led_endless`, `stop_led`) commands are emitted as requests
on the widget's own `deck_widget_surface_v1`. The compositor receives them, identifies the widget from the Wayland
connection (`SO_PEERCRED` plus the per-connection surface object), and forwards to BMC tagged with the correct widget.
The WASM module cannot influence which widget the request is attributed to: it calls `host_play_sound(name)`, the host
enqueues a Wayland request on this slot's surface only, and the compositor — not the host — applies the identity. There
is no shared audio/LED channel the host writes into, so cross-widget impersonation has nowhere to inject.

### Accepted trade-offs (latency, not correctness)

These are head-of-line effects, not data leaks:

- **Render time.** Widgets render serialized on the single context; a slow widget delays others within a tick. Fuel cap
  bounds the delay.
- **HTTP connection-pool contention.** Widgets share a `reqwest::Client` pool; widget A burst can transiently delay
  widget B's connect. Per-widget concurrent-fetch cap bounds the burst.
- **Reactor scheduling.** Tokio's cooperative scheduler ensures fairness bounded by per-widget concurrency caps.

These cost predictable latency, not correctness or isolation.

## Compositor changes

Small and additive:

- `bmc-widget-protocol`: add `lifecycle_state` enum and `lifecycle` event. Regenerate Rust bindings.
- `bmc-openwrt/src/compositor/widget_tracker.rs`: derive lifecycle state from existing scene/active/neighbor logic; emit
  `lifecycle` on change.
- `bmc-openwrt/src/compositor/protocol/dispatch.rs`: add `send_lifecycle()` helper on the per-widget surface dispatcher.
  PID-based identity logic (`SO_PEERCRED`, `set_widget_pid`, pending-connection buffer) is unchanged.
- `bmc-mock/src/mock_compositor.rs`: emit `lifecycle(visible)` once after the configure batch.

The compositor learns nothing new about widget topology.

## Error handling

| Failure                                                                                                      | Effect                                                                                                                                                                           |
| ------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| WASM widget panic (caught by `catch_unwind` around `slot.render()`) / fuel exhaustion / `RenderStatus::Dead` | Host drops the slot; thin process sees EPIPE and exits; coordinator restarts. Other slots unaffected.                                                                            |
| Wayland disconnect for one widget                                                                            | Slot dropped (same path as above).                                                                                                                                               |
| Control socket EPIPE from thin                                                                               | Slot dropped.                                                                                                                                                                    |
| EGL context loss                                                                                             | Host exits non-zero; every thin wrapper sees EPIPE; cascade restart drives a fresh host.                                                                                         |
| Pool ceiling hit on a visibility transition                                                                  | Apply deterministic fallback from "Memory-pressure behavior at pool ceiling": preserve last valid frame when possible, mark slot `resource_blocked`, and retry on release/timer. |
| Host fails to start                                                                                          | First thin wrapper exits non-zero after socket-wait timeout; coordinator's existing widget-restart backoff handles retries.                                                      |
| Bind race during host startup                                                                                | Loser exits; winner serves all clients.                                                                                                                                          |

## Migration / rollout

1. **Protocol + compositor change**, shippable on its own: add `lifecycle` event, wire it into `WidgetTracker`,
   regenerate bindings. Old widgets ignore the event; no behavior change.
2. **Host runtime + thin wrapper**: introduce `bmc-wasm-host` and `bmc-wasm-thin`. Update the coordinator's
   widget-wrapper script to exec `bmc-wasm-thin` instead of `widgets-wasm`. Remove `widgets/wasm/`. Native non-WASM
   widgets (`widgets/flip-clock`, `widgets/digital-clock`) are unaffected — they keep using the 1:1 `bmc-widget`
   wrapper.

A `BMC_WASM_NO_HOST=1` escape hatch in the thin wrapper falls back to in-process execution (today's behavior). Kept for
one release cycle, then deleted.

## Alternatives considered

### Alternative A — thin wrapper as lifetime witness (chosen)

This is what the rest of the document specifies: one OS process per widget that exists only to hold the Wayland
connection (so `SO_PEERCRED` continues to give the compositor a unique PID per widget) and a control socket (so EPIPE on
thin death tells the host to drop the slot). The host runtime is a separate, long-lived daemon, that is started by the
first widget and quit by last widget exiting.

Trade-offs:

- **Kernel-enforced identity.** The compositor's existing `SO_PEERCRED`-based widget identity logic is reused verbatim.
  No new authentication mechanism. A widget cannot impersonate another even if the host itself is buggy, because the
  kernel sets peer credentials at `connect()` and they cannot be overwritten.
- **Coordinator's mental model unchanged.** Killing a widget is still killing its PID. The BMC coordinator's supervision
  tree, restart-backoff logic, and cleanup paths are unaffected.
- **Compositor changes are additive only** (new `lifecycle` event). No reshape of the per-widget identity model.
- **Residual memory cost.** Each thin process holds its own RSS — minimal (no EGL, no Renderer, no Tokio reactor) but
  non-zero, on the order of hundreds of KB per widget after dynamic-linker overhead.
- **Per-widget PID exists.** External tools (`ps`, monitoring, oom-killer triage) can still see "one process = one
  widget" — useful for debuggability.

### Alternative B — compositor-issued secret, thin process exits

The thin wrapper is a one-shot initializer rather than a lifetime witness:

1. The BMC coordinator asks the compositor for a fresh per-widget secret (a random nonce) at widget-spawn time. The
   compositor records the secret against the widget's `instance_id` internally.
2. The coordinator passes `secret` + `wasm_path` to the thin wrapper as env vars / args.
3. The thin wrapper connects to the host (spawning it under the existing flock dance if absent), sends
   `{secret, wasm_path}` on the control socket, and exits.
4. For each new widget, the host opens its own Wayland connection (one per widget, exactly like Alt A — each widget
   still has its own `wl_display` and event queue). On that connection, the host calls a new request
   `attach_to_widget(secret)` on `deck_widget_manager_v1` before any other protocol traffic. The compositor verifies the
   secret, recovers the bound `instance_id`, binds the resulting `wl_surface` to that instance, and invalidates the
   secret (single use). Subsequent requests follow the existing `deck_widget_v1` protocol (configure, params, settings,
   configure_done arrive over Wayland as today).

The widget topology is otherwise identical to Alt A: one Wayland connection per widget; per-widget event queues;
per-widget surface disconnect cleanup. The only thing that changes is who establishes identity on the connection.

Differences from Alt A:

- **Identity by secret, not by `SO_PEERCRED`.** All connections originate from the host's PID, so `SO_PEERCRED` no
  longer disambiguates widgets. The compositor must accept multiple Wayland clients from the same PID and key identity
  on `attach_to_widget(secret)` instead.
- **Per-PID logic in the compositor is reshaped.** `pending_connections` (currently keyed by PID, used to buffer
  surface-creation until the coordinator's `set_widget_pid` arrives) is replaced by a secret table populated when the
  coordinator requests a secret. Identity arrives in band as an explicit request, so no buffering is needed.
- **No persistent thin process.** RSS overhead from the thin wrappers vanishes (~zero steady-state cost beyond the host
  itself).
- **Coordinator stop semantics change.** Killing a PID no longer stops a widget. The coordinator must use only shutdown
  through deck_widget_v1. Crash detection inside the host needs an explicit notification path from host → coordinator.
- **Compositor work is larger.** The compositor must:
  - Generate and track per-widget secrets (new state in `DeckWidgetProtocolState`).
  - Implement `attach_to_widget` request with secret validation and single-use invalidation.
  - Drop the assumption that one PID corresponds to one widget; accept N concurrent Wayland clients sharing the host's
    PID.
  - Provide new IPC for "stop widget X" since killing PIDs no longer works.

### Comparison

| Dimension                 | Alt A (chosen)                            | Alt B (alternative)                                        |
| ------------------------- | ----------------------------------------- | ---------------------------------------------------------- |
| Per-widget OS PID         | Yes (the thin wrapper)                    | No                                                         |
| Compositor identity       | `SO_PEERCRED` (existing)                  | Secret-based (new)                                         |
| Compositor changes        | Additive (`lifecycle` event)              | Reshape (secret table, multi-widget per PID, new stop IPC) |
| Coordinator stop          | `kill(pid)` (existing)                    | New IPC required                                           |
| Steady-state RSS          | One thin process per widget               | Zero (only host)                                           |
| Crash isolation signals   | Per-widget (PID death)                    | Aggregate (host PID death)                                 |
| External-tool granularity | Per-widget                                | Per-host only                                              |
| Impersonation defence     | Kernel (peer creds)                       | Cryptographic (secret entropy + single use)                |
| Migration disruption      | Low (compositor identity logic untouched) | Higher (compositor & coordinator both change shape)        |

## Testing

- **Unit tests in `bmc-wasm-host`:** slot table operations, pool acquire/release, lifecycle state-machine reducer (pure
  function, no GL).
- **Unit tests in `bmc-wasm-runtime`:** existing tests carry over after the `HostState` split; new tests cover the
  `with_renderer` guard including panic safety.
- **Integration test (host, headless):** spawn `bmc-wasm-host` against Mesa softpipe (or whichever software EGL the CI
  image carries), connect via a stub thin wrapper, load a small test WASM that draws a known pattern, read back the
  DMA-BUF, assert the pattern. Repeat with two widgets at once to verify non-interference.
- **End-to-end on device:** existing widget integration tests run unchanged.
- **Memory benchmark:** measure RSS and CMA before/after under a fixed scene with N WASM widgets. Target: total RSS
  scales sub-linearly with widget count; the per-widget RSS delta drops by at least the 7 MB EGL init overhead.

## Implementation stages

### Stage 1: `egl::EglState` split in `bmc-widget`

**Goal**: Decouple "owns EGL context" from "owns export ring + staging FBO" so the host can own one of the former and N
of the latter. **Success criteria**: existing `widgets/flip-clock`, `widgets/digital-clock` still build and run with no
behavior change; new `WidgetExportRing` type constructible from a borrowed `EglContext`. **Tests**: existing widget
tests pass; new unit test constructs two `WidgetExportRing`s against one `EglContext`. **Status**: Not Started

### Stage 2: `lifecycle` protocol event + compositor wiring

**Goal**: Ship the protocol extension and compositor-side state derivation without any host runtime changes. Old widgets
ignore the event. **Success criteria**: regenerated bindings build; `WidgetTracker` emits `lifecycle` on scene
transitions; debug logging on the widget side shows correct state sequence under manual scene cycling. **Tests**: unit
test for the `WidgetTracker → lifecycle_state` reducer; manual on-device check of the event stream. **Status**: Not
Started

### Stage 3: `bmc-wasm-runtime` refactor — split `HostState`, remove

`Renderer` ownership

**Goal**: `WasmWidgetRuntime::new` takes a shared `glow::Context` and does not construct a `Renderer`. `HostState` holds
a `*mut Renderer` set by the `with_renderer` guard. Per-widget I/O maps and shared engines are separated. **Success
criteria**: standalone WASM widget continues to render correctly when run with a synthetic single-widget host that
mimics the eventual main loop. **Tests**: existing wasmi import tests; new test for `with_renderer` guard including
panic safety. **Status**: Not Started

### Stage 4: `bmc-wasm-host` daemon — single-widget mode

**Goal**: Ship the daemon binary with a working main loop, control socket, fd handoff, and lifecycle handling — but only
one widget at a time. The framebuffer pool has a single entry. **Success criteria**: a stub thin wrapper drives a real
WASM widget through the daemon end-to-end on device; lifecycle transitions produce correct render output. **Tests**:
headless integration test (Mesa softpipe). **Status**: Not Started

### Stage 5: `bmc-wasm-thin` wrapper + coordinator integration

**Goal**: Replace `widgets/wasm/` with the new thin wrapper. Coordinator unchanged. **Success criteria**: device boot
brings up WASM widgets via the host runtime; killing the thin process tears down the widget; restart works. **Tests**:
on-device scene cycle; thin-process kill test. **Status**: Not Started

### Stage 6: Multi-widget pool + lifecycle-driven framebuffer allocation

**Goal**: Pool sized to ≥ max concurrent visible; `prepared`/`entering`/ `visible`/`leaving` correctly acquire and
release rings. **Success criteria**: measured RSS and CMA improvement under a scene with N≥4 WASM widgets vs. baseline.
**Tests**: memory benchmark; soak test cycling visibility for 10 minutes. **Status**: Not Started

### Stage 7: Cleanup

**Goal**: Remove the `BMC_WASM_NO_HOST` escape hatch once stages 4–6 are proven on device. The single-entry pool from
stage 4 is already replaced by the multi-entry pool in stage 6 — no separate removal needed. **Status**: Not Started

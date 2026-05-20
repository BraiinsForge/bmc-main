# BDK-437 — WASM host runtime: N widgets per host process

## Motivation

The Braiins Deck has 250 MB of RAM, of which 132 MB is reserved for CMA (GPU/DMA buffers). Each WASM widget today runs
as a standalone OS process: it creates its own EGL context, its own `glow::Context`, its own femtovg `Renderer`, its own
thread-per-resource I/O fleet (one `std::thread` per in-flight `ureq` fetch, per `tungstenite` WebSocket, per raw
`TcpStream`, per `mdns_sd::ServiceDaemon` registration, per SSDP search, per UDP broadcast, per HTTP listener), and its
own DMA-BUF export ring. EGL initialization plus the first render allocate roughly 7 MB of RSS per process (measured via
`/proc/<pid>/smaps_rollup`, anon + private). With N widgets, the cost scales linearly and consumes a large fraction of
available memory.

This document specifies a refactor where one host runtime process can host N WASM widgets simultaneously, sharing every
heavy resource (EGL context, renderer, font cache, and — after Stage 7 — a single shared Tokio reactor with shared HTTP
and mDNS clients) while preserving the existing per-widget OS-process identity that the compositor and the BMC
coordinator depend on.

A secondary goal: framebuffers (DMA-BUF buffers and their staging FBOs) are held only while a widget is on-screen or
about to be. A new lifecycle protocol on `deck-widget-v1` drives the wake-up / dormancy transitions. Each slot owns its
own double-buffered render target while it holds one; cross-widget pooling is deferred (§ Cross-widget pooling: deferred
contingency) because dogfooding on the native flip-clock widget showed that allocate-on-wake / free-on-dormant via the
existing GBM/EGL path is fast enough on etnaviv that a shared pool buys nothing in the steady state.

## Non-goals

- Cross-widget GL resource sharing beyond what falls out of a shared context (no texture atlasing, no shared shader
  programs beyond the blit).
- Dropping wasmi `Store` state for cold widgets. Dormant widgets keep their WASM state warm; only render targets are
  freed.
- Replacing the wasmi engine, the femtovg renderer, the Wayland protocol, or any compositor scene logic.
- Multi-host topologies. The design permits more than one host (versioned socket paths support parallel installs) but is
  optimized for a single global host.

## Architecture

```
                                  ┌──────────────────────────────┐
   coordinator (bmc/widget)       │  bmc-wasm-host (daemon)      │
        │                         │                              │
        │ spawn(bmc-wasm-thin     │  - one EGL context           │
        │       --wasm X.wasm)    │  - one Renderer / font cache │
        ▼                         │  - shared blit shader        │
   ┌────────────┐                 │  - shared I/O reactor [S7]   │
   │ bmc-wasm-  │  ctrl socket    │  - per-slot render targets   │
   │   thin     │ ──SCM_RIGHTS──► │  - N WidgetSlot              │
   │ (PID-X)    │  (wayland fd +  │      ├ wasmi Store           │
   │            │   wasm path)    │      ├ wl_display + surface  │
   │ idle       │                 │      ├ render target (opt)   │
   └────────────┘                 │      └ per-widget I/O regs   │
        │                         │                              │
        │ also opens              │  listens on                  │
        ▼ Wayland connection      │ /run/bmc/wasm-host-sdk-v{N}.sock │
   ┌────────────┐                 └──────────────────────────────┘
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

| Crate                 | Role                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Lifetime                                                                                                                   |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `bmc-wasm-host` (new) | Daemon binary. Owns EGL context, `Renderer`, slot table; gains a shared Tokio reactor + `reqwest::Client` + shared `mdns_sd::ServiceDaemon` in Stage 7. Each slot owns its own double-buffered DMA-BUF export against the shared EGL context. Listens on `/run/bmc/wasm-host-sdk-v{major}.sock`, where `{major}` is the WASM widget SDK major this host supports.                                                                                                                                                                                                                                    | One process per WASM widget SDK major version, started lazily by the first thin wrapper for that SDK that finds no socket. |
| `bmc-wasm-thin` (new) | Thin wrapper binary. Connects to Wayland, sends fd + load command to host, idles as lifetime witness.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | One process per widget instance, spawned by the BMC coordinator.                                                           |
| `bmc-wasm-runtime`    | Refactored: `WasmWidgetRuntime::new` no longer constructs the `Renderer`; the caller owns it and reaches the runtime per-frame via `with_renderer` (§ Renderer access from host functions). Initial multi-widget host keeps today's per-runtime sync I/O machinery (thread-per-fetch via `ureq`, per-WS / per-socket / per-mDNS / per-SSDP / per-UDP / per-HTTP-listener worker threads with `mpsc` channels owned by `HostState`); the `HostServices` trait split that consolidates these onto a shared Tokio reactor + `reqwest::Client` + shared `mdns_sd::ServiceDaemon` is deferred to Stage 7. | Library used by `bmc-wasm-host`.                                                                                           |
| `bmc-widget`          | `egl::EglState` split into `EglContext` (singleton, owns `/dev/dri/renderD128` fd + `GbmDevice` + EGL display/context) plus `WidgetExportBuffer` (per DMA-BUF, owning the staging FBO + stencil RBO). Existing native widgets keep using a thin owns-both wrapper that preserves today's 1:1 behavior.                                                                                                                                                                                                                                                                                               | Library.                                                                                                                   |
| `widgets/wasm/`       | Deleted as part of this work — its in-process renderer is replaced by `bmc-wasm-host` + `bmc-wasm-thin`. No fallback path is kept.                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Removed.                                                                                                                   |

Socket path uses the **WASM widget SDK major version** (`sdk-v{major}`), not the thin control-protocol version. This is
the compatibility boundary that matters for "can this widget run on this host": a host process is built for one SDK
major and accepts widgets for that SDK line. It moves independently of:

- The host binary version (a host update that preserves the SDK compatibility line does not bump it).
- The thin control protocol as long as that protocol remains compatible within the SDK line.

The host still validates `__bmc_sdk_version` during per-widget setup as defense in depth. If a future SDK major needs to
run alongside the current one, it gets its own socket/lockfile path and therefore its own host process. Minor/patch
control-protocol changes must remain backwards-compatible within a given SDK major; if the thin/host control protocol
must break without an SDK bump, that needs an explicit migration plan rather than silently reusing the same SDK socket.
The SDK major comes from `bmc_wasm_protocol::SDK_VERSION.0`; it is currently `0`.

## Lifecycle

### Thin wrapper startup

1. Parse `--wasm <path>`.
2. Open the Wayland socket directly with `std::os::unix::net::UnixStream::connect("$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY")`
   and keep the raw fd. **Do not instantiate `wayland_client::Connection` or any higher-level wayland-client object** —
   those construct an event queue and a `wl_display` sync proxy and may queue protocol traffic that would later confuse
   the host's own handshake. Peer credentials on the socket are latched by the kernel at `connect()` time and reflect
   the thin's PID/UID/GID; when the compositor later samples `SO_PEERCRED` on the server end (during
   `get_widget_surface` dispatch, as today), it sees the thin process even after the fd has been passed to the host.
3. Try to connect to `socket_path(sdk_major)` = `/run/bmc/wasm-host-sdk-v{sdk_major}.sock`. On success, jump to step 5.
4. On `ECONNREFUSED` / `ENOENT`: open `lockfile_path(sdk_major)` = `/run/bmc/wasm-host-sdk-v{sdk_major}.lock` with
   `O_CREAT | O_RDWR`, then try `flock(LOCK_EX | LOCK_NB)`.
   - If the exclusive lock is busy, another thin or initializing host owns startup. Open a fresh close-on-exec fd for
     the same lockfile and wait with bounded `flock(LOCK_SH | LOCK_NB)` retries (default 10 s; override via
     `BMC_WASM_HOST_WAIT_MS`). When `LOCK_SH` succeeds, close it and attempt one final `connect()`. Success proceeds to
     step 5; `ENOENT` / `ECONNREFUSED` means the host died during initialization and the thin exits non-zero.
   - If the exclusive lock is acquired, re-attempt step 3. If that succeeds, close the lock fd and proceed. If it still
     fails with `ECONNREFUSED` / `ENOENT`, double-fork-spawn a detached `bmc-wasm-host`, intentionally passing that
     inherited exclusive-lock fd as `--release-lock-fd <fd>`. The thin closes its own copy immediately after forking the
     host, then waits on a fresh `LOCK_SH` fd as above. The lockfile path is never unlinked by the thin. The inherited
     exclusive lock is a host-readiness barrier: the host releases it only after bind/listen and heavyweight
     initialization have completed.
5. Send `Hello { wasm_path }` on the control socket, passing the Wayland fd via `SCM_RIGHTS` in the same `sendmsg`, then
   `close(2)` the thin's own copy of the Wayland fd (the host now owns the only live duplicate; not closing here delays
   the compositor's disconnect signal until the thin exits). The widget identity travels with the Wayland connection
   (set by the compositor via `SO_PEERCRED`), so the thin wrapper does not carry it on the control socket; `params` and
   settings arrive on the host's side via the Wayland configure batch.
6. Read `Ack` with a bounded timeout (default 10 s; override via `BMC_WASM_HOST_ACK_WAIT_MS`). On timeout, log and exit
   non-zero. On `Err(msg)`, log and exit non-zero. On `Ok`, idle.
7. Block reading the control socket. On EOF (`read(2)` returns 0) / `POLLHUP` / SIGTERM, exit cleanly.

The thin wrapper accepts `--host-socket <path>` to override the canonical path. Useful for tests and `bmc-mock`. When
overridden, the lockfile path is derived from the selected socket path: replace a trailing `.sock` with `.lock`, or
append `.lock` if the socket filename has no `.sock` suffix. The host uses the same derivation to validate
`--release-lock-fd`.

### Host startup (lazy)

The host is started by the thin wrapper's double-fork (§ Host daemonization). It runs as a session leader detached from
the thin's process group, with stdio redirected to `/dev/null` and journald taking logs.

The host does not open the spawn lockfile by path. When it is spawned by a thin that won the startup election, it
inherits that thin's exclusive-lock fd via `--release-lock-fd <fd>` and holds it until it is ready to accept widgets. A
host started manually without this fd skips the readiness-lock release step and relies only on bind-race handling.

1. Bind the socket. On `EADDRINUSE`, try to `connect()` to it; if `connect()` succeeds another host is alive — exit
   (loser of the spawn race). If `connect()` fails with `ECONNREFUSED`, treat the socket as stale: `unlink()` it and
   retry the bind once. If the retry still fails, exit non-zero.
2. Bring the control socket listener up **before** any heavyweight initialization. Waiting thins are still blocked on
   the readiness lock at this point, so they do not connect until initialization succeeds, but early `bind`/`listen`
   keeps bind-race handling deterministic and keeps the stale-socket window short.
3. Initialize `EglContext` (opens `/dev/dri/renderD128`, creates `GbmDevice`, EGL display + context), `glow::Context`,
   `Renderer`, blit shader, font cache.
4. Start the Tokio `current_thread` runtime (see § Tokio integration).
5. If `--release-lock-fd` was provided, validate that the fd refers to the expected lockfile (`fcntl(F_GETFD)` plus
   `fstat(fd)` vs. `stat(lockfile_path)`), set `FD_CLOEXEC` on it, and close it immediately before entering the main
   loop. This releases the readiness barrier and wakes waiting thins.
6. Enter the main loop (§ Render orchestration).

The host stays alive after the last widget disconnects for a brief grace period (100 ms) and then exits. This is not a
"broken state": all widgets exit on shutdown together, so during that window there is by definition no widget that would
want to connect. The grace exists purely to avoid pointless respawn churn if a fresh widget races the last widget's exit
by a few milliseconds.

#### Host daemonization

The host is detached from the thin wrapper that spawned it:

1. The thin wrapper calls `fork()`. The child calls `setsid()` (new session, no controlling terminal), then `fork()`
   again; the intermediate child exits immediately. The grandchild becomes the host, reparented to PID 1.
2. The grandchild closes every inherited application fd except the lockfile fd intentionally passed as
   `--release-lock-fd <fd>`; Wayland fds, host control sockets, self-pipe fds, and unrelated descriptors are closed or
   close-on-exec. It redirects stdio to `/dev/null`, then `exec()`s
   `bmc-wasm-host --host-socket <path> --release-lock-fd <fd>`.
3. The thin wrapper's original process reaps the intermediate child with `waitpid()` so it does not become a zombie.

The host exits when the last control socket connection closes (after the grace period above); it does not need to
inherit any state from the thin wrapper that spawned it.

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

- Control socket peer-closed (host's `read(2)` returns 0 or `poll(2)` reports `POLLHUP` / `POLLRDHUP`; thin process
  exited).
- Wayland disconnect for this widget.
- `runtime.render()` returning `RenderStatus::Dead`.

Teardown: abort all per-widget Tokio tasks (fetches, websockets, sockets, mDNS browses, SSDP, UDP, HTTP listeners); drop
the `WasmWidgetRuntime`; drop the slot's render target (if any), which destroys its export buffers and `wl_buffer`
proxies; close the per-widget Wayland fd; close the control socket if still open; remove the slot from the table.

## Control protocol (thin ↔ host)

`AF_UNIX` `SOCK_STREAM`. The socket path is selected by WASM widget SDK major, so the thin/host control protocol must
remain compatible within that SDK line. There is no in-band negotiation in Stage 6.

### Wire format

The protocol carries exactly two message shapes — `Hello` and `Ack` — so framing is hand-rolled rather than pulled
through `bincode` or any serde stack. No external serialization dependency, no version-drift risk, one screen of code on
each side.

All integers are little-endian. All string fields are encoded as `[u32 LE length][UTF-8 bytes]`. Length is bounded:
strings longer than 64 KiB are a protocol error and cause the receiver to close the connection.

#### `Hello` (thin → host, sent once immediately after `connect()`)

```
+--------+--------+
| u8 tag | path   |
| = 0x01 | string |
+--------+--------+
```

`tag` exists only to leave room for future thin-side messages (currently none). The Wayland fd is carried out-of-band
via `SCM_RIGHTS` ancillary data on the same `sendmsg(2)`; it is not part of the wire payload.

#### `Ack` (host → thin, sent once in response)

```
+--------+----------------------+
| u8 tag | optional err message |
+--------+----------------------+
  0x00 = Ok            (no payload follows)
  0x01 = Err(message)  (message: UTF-8 string per the encoding above)
```

#### Rust types (illustrative)

```rust
// In bmc-wasm-thin-protocol (a small shared crate with no dependencies).
pub enum HelloMsg { Load { wasm_path: String } }
pub enum AckMsg   { Ok, Err(String) }

// fn write_hello<W: Write>(w: &mut W, msg: &HelloMsg) -> io::Result<()> { … }
// fn read_hello<R:  Read>(r: &mut R) -> io::Result<HelloMsg>            { … }
// fn write_ack  <W: Write>(w: &mut W, msg: &AckMsg)   -> io::Result<()> { … }
// fn read_ack   <R:  Read>(r: &mut R) -> io::Result<AckMsg>             { … }
```

The reader rejects strings longer than 64 KiB before allocating.

After `Ack::Ok` no further messages flow. The channel exists purely as a lifetime witness. The host watches its end with
`poll(2)` and treats either `read(2) == 0` (EOF) or `POLLHUP` / `POLLRDHUP` as "thin gone". (Note: `EPIPE` is a
write-side error and does not appear on this idle read path; implementations that key only on `EPIPE` will leak slots.)

### FD ownership

The Wayland fd is sent via `SCM_RIGHTS` ancillary data on the same `sendmsg(2)` call that carries the `Hello` frame. The
thin wrapper `close(2)`s its own descriptor immediately after the send returns; the host owns the only live duplicate.
The host closes its descriptor when the slot is torn down.

### Sizing

`wasm_path` is a path, not the module bytes, so the thin wrapper's RSS stays minimal. The host opens the file directly.

## Render orchestration

### Resource ownership

| Resource                                                                                                                                                                              | Owner                                                  | Notes                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `EglContext`, `glow::Context`                                                                                                                                                         | Host (singleton)                                       | One context made current at host start, never switched.                                                                                                                                                                                                                                                                 |
| `Renderer` (femtovg `Canvas`, fonts, paths)                                                                                                                                           | Host (singleton)                                       | Accessed by host functions during `slot.render()` via `NonNull<Renderer>` set on the slot's `HostState`.                                                                                                                                                                                                                |
| `SharedRenderScratch` (staging FBO + stencil RBO + Y-flip blit program/VBO)                                                                                                           | Host (singleton)                                       | One staging FBO sized to the display maximum, reused by every slot's WASM render path under the single-threaded render model (§ Renderer access from host functions). Allocated once at host start against the shared `EglContext`; lives for the host's lifetime. See Stage 2.5 below.                                 |
| Per-widget I/O worker threads (`ureq` fetch, `tungstenite` WS, raw `TcpStream` socket, `mdns_sd::ServiceDaemon` per registration, SSDP, UDP, HTTP listener) and their `mpsc` channels | Per-widget runtime (`HostState` / `WasmWidgetRuntime`) | Initial host keeps today's per-`WasmWidgetRuntime` machinery verbatim. Resource consolidation onto a single shared Tokio reactor + `reqwest::Client` + shared `mdns_sd::ServiceDaemon` is deferred to Stage 7 (§ Implementation stages). Per-widget concurrency caps are not enforced until 7 lands.                    |
| `WasmWidgetRuntime` (wasmi `Store`, linear memory)                                                                                                                                    | Per-widget slot                                        | Lives for the slot's full lifetime, including `dormant`.                                                                                                                                                                                                                                                                |
| `wl_display`, `wl_surface`, event queue                                                                                                                                               | Per-widget slot                                        | One connection per widget.                                                                                                                                                                                                                                                                                              |
| DMA-BUF export buffers (`ExportBuffer` × 2)                                                                                                                                           | Per-widget slot                                        | Allocated on `dormant → {prepared, entering, visible, leaving}` wake-up against the shared `EglContext`; destroyed on `{prepared, entering, visible, leaving} → dormant` (or slot drop). Re-allocation on the next wake produces a fresh `wl_buffer` ObjectId; the compositor pays a one-time EGLImage import per wake. |
| `wl_buffer` proxies                                                                                                                                                                   | Per-widget slot                                        | Minted via `zwp_linux_dmabuf_v1.create_params` + `create_immed` on the slot's own Wayland connection, paired one-to-one with the slot's export buffers, destroyed together with them.                                                                                                                                   |

### Renderer access from host functions

All renderer access today is bracketed by `runtime.renderer().begin_frame(…)` and `runtime.renderer().flush()` around
`runtime.render(delta_ms)`. Host import modules other than `render.rs` do not touch the renderer. Async I/O delivery
(`deliver_fetch_responses`, `deliver_ws_messages`, …) does not touch the renderer.

The host owns the `Renderer` on its stack for the entire main-loop iteration and passes it into per-slot work as a
`NonNull<Renderer>`, not as `&mut Renderer`. The unsafe conversion happens **once**, at a single well-defined boundary
in the host loop, and no Rust `&mut Renderer` parameter is live on any function frame while the pointer is parked in a
slot's `HostState`. This avoids the Stacked / Tree Borrows hazard of holding a parent `&mut` reference and aliasing it
through a stored pointer.

#### The boundary

```rust
// In the host main loop. `renderer` is a stack-local owned Renderer.
let renderer_ptr = NonNull::new(core::ptr::addr_of_mut!(renderer))
    .expect("BUG: addr_of_mut! cannot produce null");
let now = Instant::now();

for slot in slots.values_mut() {
    if !slot.needs_render(now) { continue; }
    let delta_ms = slot.tick_delta(now);
    // SAFETY: `renderer` is on this stack frame and no other &mut Renderer exists
    // for the duration of this call. `slots` iteration is sequential, so only one
    // slot at a time observes `renderer_ptr`.
    slot.render(renderer_ptr, delta_ms)?;
}
```

`addr_of_mut!` does not create an intermediate `&mut Renderer` reference, so the parent reference does not appear on the
borrow stack at all. The only entity referencing the renderer is the raw pointer.

#### Slot and runtime

```rust
impl WidgetSlot {
    fn render(&mut self, renderer: NonNull<Renderer>, delta_ms: u32) -> Result<RenderStatus> {
        // SAFETY: caller contract — `renderer` is valid, exclusive, and no other
        // &mut Renderer reference is live. Each `as_mut()` produces a fresh reborrow
        // that lives only until the next `;`.
        unsafe { renderer.as_ptr().as_mut() }
            .expect("BUG: renderer non-null")
            .begin_frame(self.width, self.height, 1.0);

        let status = self.runtime.with_renderer(renderer, |rt| rt.render(delta_ms))?;

        unsafe { renderer.as_ptr().as_mut() }
            .expect("BUG: renderer non-null")
            .flush();

        Ok(status)
    }
}

impl WasmWidgetRuntime {
    pub fn with_renderer<R>(
        &mut self,
        renderer: NonNull<Renderer>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.store.data_mut().renderer_ptr = Some(renderer);
        let result = f(self);
        self.store.data_mut().renderer_ptr = None;
        result
    }
}
```

`with_renderer` never takes `&mut Renderer`. The pointer is installed in `HostState` before `f` runs and cleared after.
The closure receives `&mut WasmWidgetRuntime` only — the renderer is reached exclusively through
`HostState::renderer_ptr` from inside host import functions.

#### Host imports

Each host import that needs the renderer calls a small helper that materializes a short-lived `&mut Renderer` from the
pointer, uses it synchronously, and drops it before returning. wasmi calls run on a single thread with no async yields
inside a host function, so two host imports cannot overlap and the reborrow lifetimes are bounded by the import call.

```rust
fn with_renderer<R>(caller: &mut Caller<HostState>, f: impl FnOnce(&mut Renderer) -> R) -> R {
    let ptr = caller.data_mut().renderer_ptr
        .expect("BUG: renderer accessed outside render scope");
    // SAFETY: ptr was installed by WasmWidgetRuntime::with_renderer on this same
    // thread and is non-aliased for the duration of this synchronous host call.
    let renderer: &mut Renderer = unsafe { ptr.as_ptr().as_mut().expect("BUG: non-null") };
    f(renderer)
}
```

#### Panic safety

If `f` panics inside `with_renderer`, `renderer_ptr` remains set in `HostState`. That is fine: the host's `catch_unwind`
wrapper around `slot.render()` drops the entire slot — including its `HostState` — immediately, so the stale pointer is
never observed again. The pointer also cannot outlive the stack frame that owns the `Renderer`: that frame is the
main-loop iteration, and the pointer is only stored in slot state that lives no longer than the slot.

A `Drop`-based clearing guard is not used because clearing on unwind has no observable benefit (the slot dies anyway)
and the simpler code is easier to audit. Host import functions that read the pointer outside a render scope panic with
`expect("BUG: renderer accessed outside render scope")` — a programming error, not a recoverable condition.

#### Verification

A unit test (`tests/with_renderer_aliasing.rs` in `bmc-wasm-runtime`) drives the `with_renderer` path with a stub
`Renderer` and is intended to be run under
`MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-strict-provenance" cargo +nightly miri test`. The test exercises: (a) normal
install/use/clear cycle, (b) two sequential `with_renderer` calls on the same slot, (c) host-import-style reborrow from
the stored pointer, and (d) the panic path (panic inside `f`, slot dropped from outside `catch_unwind`). Miri is run on
this test in CI as a separate optional job; if Miri flags UB here, the pattern is wrong and the design must be revisited
before Stage 5 ships.

### Per-runtime I/O integration

In the initial multi-widget host (Stages 5–6) there is **no Tokio reactor**. Each `WasmWidgetRuntime` carries its
existing per-resource worker threads — one `std::thread::spawn` per in-flight `ureq` fetch, per `tungstenite` WebSocket,
per raw socket, per `mdns_sd::ServiceDaemon`, per SSDP search, per UDP broadcast, per HTTP listener — and its existing
`mpsc` channels rooted in `HostState`. Results land in `HostState` synchronously via `WasmWidgetRuntime::deliver_*`
calls driven from the host's main loop (§ Main loop).

The reasons for keeping this shape initially rather than introducing a shared reactor:

- The wasmi `Store`, `HostState`, and `Renderer` are not `Send`. Each runtime drives its own delivery synchronously from
  the host's main thread; no cross-thread plumbing of `HostState` is needed.
- Each runtime already owns its delivery channels and is already structurally per-widget — the host just creates N
  runtimes and ticks each.
- Consolidating the per-widget worker threads onto a shared Tokio reactor + `reqwest::Client` + shared
  `mdns_sd::ServiceDaemon` is the goal of Stage 7; it is deferred because the rendering-pipeline consolidation (Stages
  3–6) is the load-bearing memory win and can ship without it.

Properties to preserve in the meantime:

- **Per-runtime shutdown unblocks workers.** Standalone today, a widget process exit lets the OS reap blocking threads
  (a `ureq.call()` mid-flight, a `TcpStream::read` mid-recv). In the host, slot teardown must explicitly drop every
  `mpsc::Sender` held in `HostState` and signal every `stop_tx` so that worker threads exit on their next channel check.
  Threads blocked inside non-cancellable syscalls (`ureq`, `tungstenite::accept`, `mdns-sd` socket reads) will linger
  until their underlying I/O completes; this is acceptable because they no longer hold any live reference to the slot's
  state once the senders are dropped.
- **`mdns_sd::ServiceDaemon` multi-instance.** Today each widget process has its own daemon binding mDNS multicast. N
  daemons in one process is uncharted territory; Stage 5 must include a smoke test that runs two runtimes each with an
  active `mdns_browse` against the same service and asserts both receive announcements. If `mdns-sd` cannot multi-bind
  5353 in a single process, Stage 5 falls back to one shared `ServiceDaemon` (a narrow precursor to 7; the rest of the
  I/O model stays per-runtime).
- **HTTP listener port collisions.** Two widgets listening on the same port in one process collide where two processes
  would have also collided, surfacing as `EADDRINUSE` on the second `bind`. No code change; documented behaviour.
- **Thread-count headroom.** N widgets × per-widget burst can produce dozens of OS threads with Rust's default 2 MB
  virtual stack each. Initial Stage 6 acceptance includes a thread-count measurement under a representative scene; if it
  threatens to overwhelm the device, Stage 7 is pulled forward.

### Main loop

```rust
let renderer_ptr = NonNull::new(core::ptr::addr_of_mut!(renderer))
    .expect("BUG: addr_of_mut! cannot produce null");

loop {
    let timeout = compute_poll_timeout(&slots, &lifetime, Instant::now());
    poll(&mut all_fds, timeout);
    let now = Instant::now();

    accept_new_thin_connections()?;

    for slot in slots.values_mut() {
        slot.dispatch_wayland_events()?;
        slot.dispatch_control_events()?;
        // Apply any queued lifecycle deltas: allocate the slot's own
        // export buffers on wake-up, destroy them on dormancy. Each
        // slot's allocate/free is local to itself, so iteration order
        // over `slots` does not matter.
        slot.apply_lifecycle(&shared.egl);
    }

    for slot in slots.values_mut() {
        // Drain the runtime's own per-resource mpsc receivers into
        // `pending_*` state. Calls into existing `WasmWidgetRuntime::deliver_*`
        // entry points — no Tokio reactor is involved (§ Per-runtime
        // I/O integration). Stage 7 replaces this with a shared
        // reactor drain.
        slot.runtime.poll_deliveries();
    }

    // No live `&mut Renderer` reference is held across this loop: only the raw
    // pointer is shared with slots. See § Renderer access from host functions.
    for slot in slots.values_mut() {
        if !slot.needs_render(now) { continue; }
        let delta_ms = slot.tick_delta(now);
        slot.render(renderer_ptr, delta_ms)?;
    }
}
```

`needs_render(now)` is true when **all** of the following hold:

- The slot's lifecycle state is in `{prepared, entering, visible, leaving}` (the render-target set; see § States).
- The slot is dirty (a fresh frame is required by the state machine, or `runtime.wants_next_frame()` returned true).
- `now - slot.last_render_at >= MIN_INTER_FRAME` for this slot. The default `MIN_INTER_FRAME` is **8 ms** (120 fps
  ceiling per slot); the cap exists to prevent a misbehaving widget that returns `wants_next_frame() == true` every
  iteration from spinning the host at 100 % CPU.
- The slot is not `resource_blocked` (allocation failure has prevented its render target from existing even though
  `target_state` is in the eligible set).

`tick_delta(now)` returns the elapsed time since this slot's last render and updates its last-render timestamp.

Rendering is strictly serialized: the EGL context is current to the host's single context; each iteration only changes
which FBO is bound. There is no `eglMakeCurrent` thrash and no thread synchronization.

### Poll timeout

`compute_poll_timeout(slots, lifetime, now) -> i32` returns the millisecond timeout for `poll(2)`, computed as the
minimum of:

- For each slot whose lifecycle state is renderable: `max(0, MIN_INTER_FRAME - (now - last_render_at))` if the slot is
  already dirty (this is the inter-frame delay that gates a re-render); otherwise the slot's `next_frame_delay()` if
  frame callbacks are enabled and the delay is positive.
- `retry_at - now` for any `resource_blocked` slot (the bounded retry timer from § Allocation failure behavior).
- 100 ms ceiling if any slot has pending async I/O (matches today's behaviour).
- Remaining post-disconnect grace window if the host has shed all slots and is waiting to exit.
- `-1` (poll(2)'s indefinite-block sentinel; **not** `i32::MAX`) if no slot contributes a finite value and we are
  neither in grace nor blocked.

This makes the timeout never zero unless a slot's inter-frame delay has already expired AND the slot is dirty, which is
exactly when an immediate iteration is warranted.

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

- **dormant** — far from active scene; no render target; runtime not ticked (but async I/O is still delivered).
- **prepared** — neighbor of active scene; render target allocated and one cached frame rendered when dirty, but runtime
  animation is not ticked (async I/O still delivered). The wasmi `Store` stays warm so a scene drag can hand an
  already-rendered buffer to the compositor.
- **entering** — on-screen transition in progress; re-render once.
- **visible** — active on-screen; full render loop with animation and frame callbacks.
- **leaving** — on-screen transition out; continue rendering until the transition completes.

| State    | wasmi Store | Async I/O | Render target | Render loop    | Frame cb |
| -------- | ----------- | --------- | ------------- | -------------- | -------- |
| dormant  | warm        | delivered | none          | paused         | —        |
| prepared | warm        | delivered | allocated     | render once    | —        |
| entering | warm        | delivered | allocated     | re-render once | —        |
| visible  | warm        | delivered | allocated     | full loop      | yes      |
| leaving  | warm        | delivered | allocated     | full loop      | yes      |

The render-target set is `{Prepared, Entering, Visible, Leaving}`. `Prepared` is pre-rendered but keeps animation and
frame callbacks paused; only `{Visible, Leaving}` participate in the continuous animation loop. `Dormant` is the sole
buffer-free state.

### Transitions

The full 5×5 transition matrix. The compositor's normal path is
`dormant → prepared → entering → visible → leaving → prepared → dormant`; other transitions are tolerated for
robustness. Each cell describes the host action.

|              | →dormant              | →prepared                               | →entering                               | →visible                                             | →leaving                                                                  |
| ------------ | --------------------- | --------------------------------------- | --------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------------------- |
| **dormant**  | no-op                 | acquire render target; render one frame | acquire render target; render one frame | acquire render target; full render; enable animation | acquire render target; full render; enable animation (treat as `visible`) |
| **prepared** | release render target | no-op                                   | keep target; render one frame           | keep target; full render; enable animation           | keep target; full render; enable animation                                |
| **entering** | release render target | keep target; render one frame           | no-op                                   | enable animation                                     | continue full loop                                                        |
| **visible**  | release render target | keep target; render one frame           | continue rendering (re-enter)           | no-op                                                | continue full loop                                                        |
| **leaving**  | release render target | keep target; render one frame           | continue full loop (re-entry mid-leave) | re-enable animation; continue full loop              | no-op                                                                     |

"Release render target" means dropping the slot's export buffers, depth/stencil RBOs, staging FBO, and their paired
`wl_buffer` proxies (see § Per-slot render targets). "Acquire render target" means allocating fresh buffers + proxies
sized to the slot's surface against the shared `EglContext`. If allocation fails (CMA exhaustion, GBM error), the slot
follows the failure rules in § Allocation failure behavior. The host keeps the compositor-requested lifecycle as the
slot's `target_state` and records `resource_blocked` until allocation succeeds or a new lifecycle event supersedes it.

The state machine is total — every (current, target) pair has a defined effect — so it tolerates arbitrary jumps emitted
by the compositor without panicking.

### Compositor source of truth

`bmc-openwrt/src/compositor/widget_tracker.rs` maps current scene state to lifecycle states:

- Active widget = `visible`.
- Immediate neighbor in the scene cycle = `prepared`.
- All others = `dormant`.
- During an active drag: outgoing = `leaving`, incoming = `entering`. On drag settle, snap to `visible`/`prepared`.

#### Batch ordering on scene swaps

When a single scene change re-maps lifecycle states for more than one widget at once — scene preview open/close, scene
insert/remove, a full scene-cycle jump — the compositor emits the resulting `lifecycle` events in two ordered batches
within the same dispatch:

1. **Release batch first.** Every transition into the buffer-free state
   (`{prepared, entering, visible, leaving} → dormant`). The compositor flushes its Wayland clients after this batch so
   the events leave the server in order.
2. **Acquire batch second.** Every transition into the buffer-allocated set
   (`dormant → {prepared, entering, visible, leaving}`). Flush again.

This ordering keeps peak CMA occupancy during a scene swap bounded by the post-swap working set rather than the union of
the pre- and post-swap working sets: outgoing widgets free their buffers before incoming widgets allocate theirs. It
also gives the eventual cross-widget pool (§ Cross-widget pooling: deferred contingency) the same guarantee for free if
and when we add it back.

The host applies lifecycle deltas inline in `apply_lifecycle` on each affected slot. Each slot's allocate/free is local
to itself, so the undefined iteration order over `slots.values_mut()` is fine on the host side; the compositor-side
batch ordering above is what makes the *aggregate* CMA peak bounded.

## Per-slot render targets

Each slot owns the export buffers and `wl_buffer` proxies it currently needs against the host's shared `EglContext`. No
cross-widget sharing. The lifecycle protocol drives allocation and release; no other policy logic exists on the host
side. Empirical justification: alternating dormant/wake cycles on the native flip-clock widget (see commits
`94b38513`/`9365d853`) show that GBM/EGL allocate-on-wake and free-on-dormant are fast on etnaviv, and the lifecycle
gating alone caps simultaneous ownership to the target-owning set (`{prepared, entering, visible, leaving}`).

### Per-slot state

A slot in a target-owning lifecycle state owns:

- Two `ExportBuffer`s (`bmc-widget::egl::ExportBuffer`): GBM BO + EGLImage + GL texture + FBO, ping-ponged each frame.
  Reused from `DoubleBufferState` (the existing double-buffer helper, made public in `bmc-widget` for the host to
  consume against a borrowed `EglContext`).
- No per-slot staging FBO or stencil RBO. Slots that render via the staging + Y-flip blit pipeline (the WASM case) share
  the host's singleton `SharedRenderScratch` (see Stage 2.5 and § Resource ownership); slots that render directly into
  the export buffer (no femtovg) do not touch the scratch.
- One `wl_buffer` proxy per `ExportBuffer`, minted on the slot's Wayland connection via
  `zwp_linux_dmabuf_v1.create_params` + `create_immed`. The proxy lives exactly as long as the underlying
  `ExportBuffer`; both are torn down together when the slot transitions to `dormant` or is dropped.

A slot in `dormant` owns none of the above; only its wasmi `Store`, host I/O handle maps, Wayland connection, and
`wl_surface` persist.

### Wake-up and dormancy

On any `dormant → {prepared, entering, visible, leaving}` transition:

1. Allocate two `ExportBuffer`s against `shared.egl` (sized to the surface's configure dimensions).
2. Allocate the staging `WidgetExportBuffer` if applicable.
3. Mint a `wl_buffer` proxy per `ExportBuffer` on the slot's connection. Register a `wl_buffer.release` listener that
   marks the slot's buffer free (drives the ping-pong's idle bookkeeping; not a return-to-pool).
4. Render the first frame per the lifecycle table.

On any `{prepared, entering, visible, leaving} → dormant` transition: drop all of the above. `ExportBuffer::destroy`
(currently `EglContext::destroy_export_buffer`) requires the EGL context current, which is always true in the host
because the context is current throughout the main loop. `wl_buffer.destroy` on each proxy fires the Wayland-side
reclaim of any in-flight attachment.

Transitions inside `{prepared, entering, visible, leaving}` keep the render target; only animation/frame-callback
behaviour flips per the lifecycle table.

### Compositor texture cache and re-import

Every wake mints fresh `wl_buffer` ObjectIds, so the compositor's `HashMap<ObjectId, GlesTexture>` cache
(`bmc-openwrt/src/compositor/scene_renderer.rs:38`) does *not* hit on the second wake. The compositor pays one EGLImage
import per `ExportBuffer` per wake, then caches. On etnaviv, EGLImage import costs ~hundreds of microseconds for our
screen-sized buffers — fast enough that the cost is invisible at the seam during a scene drag. If profiling later
contradicts this, the cross-widget pool (below) restores per-widget cache warmth.

### Allocation failure behavior

CMA is bounded (132 MB total, shared with the compositor and other consumers). A `gbm_bo_create` or `eglCreateImage`
failure is rare but possible. The slot's response is local:

Failure can only occur on transitions into the buffer-allocated set `dormant → {prepared, entering, visible, leaving}`
(the host does not allocate on transitions inside the target-owning set). Failure response is local:

- **`dormant → prepared` or `dormant → entering` failure:** keep slot non-rendering; mark `resource_blocked`; do not
  emit partial transition commits.
- **`dormant → visible` or `dormant → leaving` failure:** if a previously-committed buffer exists (e.g., from a prior
  visible cycle that was torn down), the surface is blank — the host does not retain buffers across `dormant`; mark
  `resource_blocked`; keep `target_state` at the compositor-requested value.
- **Already in the target-owning set with an existing target:** continue rendering on current target; no forced drop.
- **Retry policy:** retry blocked slots on a bounded timer (1 s). Allocation failures are not driven by a release event
  in the no-pool model because there is no shared release queue; the timer is the only retry trigger.

Observability:

- Emit rate-limited warning logs with widget identity, requested size, and lifecycle state on allocation failure.
- Expose counters for `alloc_fail`, `blocked_slots`, and `blocked_duration_ms`.

The aggregate worst-case CMA footprint under the lifecycle protocol is bounded by
`|{entering, visible, leaving}| × 2 buffers × bytes_per_buffer`. With the compositor's batch ordering on scene swaps (§
Batch ordering on scene swaps), peak occupancy during a swap stays at the post-swap working set, not the union. At
today's resolution (1280×480 ARGB8888 ≈ 2.34 MB per full-display buffer; widgets at smaller surface sizes cost
proportionally less), the steady-state worst case during a scene drag — outgoing widget in `leaving` plus incoming
widget in `entering` — is approximately 2 surfaces × 2 buffers × 2.34 MB ≈ 9.4 MB if both are full-display; ~4.7 MB when
only one widget is `visible`.

## Cross-widget pooling: deferred contingency

The `bmc-wasm-buffer-pool` crate (commits `3eb672c7`…`9e698908`) implements a generic pool with affinity-based reuse,
priority-based steal, byte-budgeted ceiling, and connection teardown. It is fully tested and intentionally kept in the
tree even though the host does not wire it up in the initial implementation. Stage 0 of this plan lays down the crate;
later stages do not depend on it.

The pool earns its complexity only if one of these becomes a problem:

- **EGLImage import latency at the seam.** If profiling on device shows that the per-wake EGLImage import for the
  incoming widget's `ExportBuffer`s causes a visible hitch during scene drags, the pool's affinity-based reuse keeps the
  same `wl_buffer` ObjectIds across a widget's on/off cycles and avoids the re-import.
- **CMA fragmentation under repeated alloc/free.** If `dmabuf_create` starts failing or returning slower over time
  because etnaviv/CMA cannot find contiguous regions after extended on/off cycling, the pool's stable allocations avoid
  the churn.
- **Aggregate CMA pressure under widget growth.** If the steady-state widget count grows so that the lifecycle-bounded
  working set approaches CMA exhaustion, the pool's hard ceiling + steal-by-priority degrades gracefully where naive
  per-slot allocation hits `ENOMEM`.

Revival path: introduce a `Pool` field on `SharedHost`, switch the slot from owning `ExportBuffer`s to holding
`EntryId`s, wire `apply_lifecycle` to `Pool::acquire` / `Pool::release`, add the two-pass release-then-acquire ordering
to the main loop (§ Batch ordering on scene swaps already has the contract on the compositor side), add the
`ResourceFactory::destroy` trait method so the pool can free CMA on `Drop`, and wire `wl_buffer.release` events to
`Pool::on_buffer_released`. The migration is mechanical because the pool crate has been kept aligned with the slot's
allocation shape.

## Host API isolation

### Initial host (Stages 5–6): per-runtime sync I/O

In the initial multi-widget host, each `WasmWidgetRuntime` keeps its existing `HostState` shape verbatim: per-resource
`mpsc` channels (`fetch_rx`/`fetch_tx`, `websockets[id].event_rx`/`msg_tx`, `sockets`, `mdns_browses`, `ssdp`, `udp`,
`http_listeners`), per-registration `mdns_sd::ServiceDaemon`, plus the `pending_*` / handle maps that today's runtime
already maintains. The host owns one such runtime per slot and ticks `runtime.poll_deliveries()` on each per iteration
(§ Main loop). Nothing in `bmc-wasm-runtime`'s I/O architecture or dependency cone (`ureq`, `tungstenite`, `mdns-sd`)
changes for Stages 4–6.

`SharedHost` in this phase owns **only** the rendering singletons (see § Resource ownership): one `EglContext`, one
`glow::Context`, one `Renderer`, one `SharedRenderScratch`, one blit shader, one shared font cache. No Tokio runtime, no
`reqwest::Client`, no shared `mdns_sd::ServiceDaemon`. The `WidgetSlotHost` struct (described below) is also not
introduced in this phase; per-slot OS handles live where they live today, inside each `WasmWidgetRuntime`.

### Deferred (Stage 7): `HostServices` trait split

The trait-based design below is the target shape after Stage 7 has consolidated I/O onto a shared reactor. It is
retained here as the design reference; Stage 7's success criteria are written against it. Until Stage 7 lands, none of
the types in this subsection exist.

The split lands across two crates. `bmc-wasm-runtime` defines `HostState` and the `HostServices` trait; it pulls in no
async runtime and no HTTP client. `bmc-wasm-host` owns the concrete impl, the Tokio reactor, `reqwest::Client`, the mDNS
daemon, and all per-slot OS-level state (tokio `JoinHandle`s, Wayland surface sender, inbox channel).

```rust
// In `bmc-wasm-runtime`. No tokio, no reqwest, no async runtime dependency.
pub struct HostState {
    // per-widget, private
    fuel_budget: FuelBudget,
    rng: XorShift32,
    /// Per-resource-class bookkeeping for results delivered back into the runtime
    /// by `WasmWidgetRuntime::deliver_*` each tick. These hold widget-visible
    /// state (pending response payloads, queued frames, last-seen sequence
    /// numbers, etc.) — not OS resources and not tokio `JoinHandle`s. Those live
    /// host-side, keyed by the same IDs.
    pending_fetches:     HashMap<FetchId,  PendingFetch>,
    pending_websockets:  HashMap<WsId,     PendingWs>,
    pending_sockets:     HashMap<SockId,   PendingSock>,
    pending_mdns_browses:   HashMap<MdnsBrowseId,       PendingMdnsBrowse>,
    pending_mdns_registers: HashMap<MdnsRegistrationId, PendingMdnsRegister>,
    pending_ssdp:        HashMap<SsdpId,   PendingSsdp>,
    pending_udp:         HashMap<UdpId,    PendingUdp>,
    pending_http_listen: HashMap<HttpId,   PendingHttp>,
    pending_audio:       HashMap<AudioId,  PendingAudio>,
    pending_led:         HashMap<LedId,    PendingLed>,
    settings:      WidgetSettings,
    last_render_target_size: (u32, u32),
    renderer_ptr:  Option<NonNull<Renderer>>,

    /// Trait object for outbound calls — fetch start/cancel, ws send/close,
    /// mdns browse, play_sound, led_temporary, … The host's impl is per-slot,
    /// so it carries this widget's identity implicitly; the runtime never
    /// names itself in calls.
    host: Rc<dyn HostServices>,
}

/// Defined in `bmc-wasm-runtime`. All methods are synchronous: they take the
/// request, return a freshly-allocated ID (or `Result<Id, HostError>`), and
/// never block on network or disk I/O. ID allocation lives on the host side —
/// each slot's `WidgetSlotHost` carries one counter per resource class. The
/// runtime stores the returned ID in the matching `pending_*` map. Background
/// work and async machinery live entirely on the host side. Results flow back
/// via the host calling `WasmWidgetRuntime::deliver_*` each tick (§ Render
/// orchestration / main loop) — the channel itself does **not** cross the
/// crate boundary.
///
/// Split into sub-traits matching the existing `runtime/imports/*.rs` layout
/// so each one can be stubbed independently in tests and in the testbed.
/// `HostServices` is a marker super-trait composed via a blanket impl, so
/// `Rc<dyn HostServices>` can dispatch any sub-trait method.
pub trait HostNetwork {
    fn fetch_start(&self, req: FetchRequest) -> Result<FetchId, HostError>;
    fn fetch_cancel(&self, id: FetchId);

    fn ws_connect(&self, req: WsConnectRequest) -> Result<WsId, HostError>;
    fn ws_send(&self, id: WsId, msg: WsMessage) -> Result<(), HostError>;
    fn ws_close(&self, id: WsId);

    fn socket_connect(&self, req: SocketRequest) -> Result<SockId, HostError>;
    fn socket_send(&self, id: SockId, bytes: &[u8]) -> Result<(), HostError>;
    fn socket_close(&self, id: SockId);

    fn udp_open(&self, req: UdpRequest) -> Result<UdpId, HostError>;
    fn udp_send(&self, id: UdpId, dst: SocketAddr, bytes: &[u8]) -> Result<(), HostError>;
    fn udp_close(&self, id: UdpId);

    fn mdns_browse_start(&self, service: &str) -> Result<MdnsBrowseId, HostError>;
    fn mdns_browse_stop(&self, id: MdnsBrowseId);
    fn mdns_register(&self, req: MdnsRegisterRequest) -> Result<MdnsRegistrationId, HostError>;
    fn mdns_unregister(&self, id: MdnsRegistrationId);

    fn ssdp_search(&self, req: SsdpRequest) -> Result<SsdpId, HostError>;
    fn ssdp_cancel(&self, id: SsdpId);

    fn http_listen(&self, req: HttpListenRequest) -> Result<HttpId, HostError>;
    fn http_respond(&self, id: HttpId, resp: HttpResponse) -> Result<(), HostError>;
    fn http_listen_close(&self, id: HttpId);
}

pub trait HostAudio {
    fn play_sound(&self, req: SoundRequest) -> Result<AudioId, HostError>;
    fn stop_sound(&self, id: AudioId);
}

pub trait HostLed {
    fn led_temporary(&self, req: LedRequest) -> Result<LedId, HostError>;
    fn led_cancel(&self, id: LedId);
}

pub trait HostServices: HostNetwork + HostAudio + HostLed {}
impl<T: HostNetwork + HostAudio + HostLed + ?Sized> HostServices for T {}
```

```rust
// In `bmc-wasm-host`. Not visible to `bmc-wasm-runtime`.
pub struct SharedHost {
    /// EGL context plus its underlying GBM device on `/dev/dri/renderD128`. Held by
    /// `EglContext` and reused for every DMA-BUF export performed by any slot. One fd,
    /// one `GbmDevice`, one `EGLDisplay`, one `EGLContext` for the whole host.
    egl: EglContext,
    glow: glow::Context,
    blit: BlitShader,
    reqwest: reqwest::Client,
    tokio: tokio::runtime::Handle,
    mdns: Arc<MdnsDaemon>,
}

/// Per-slot host-side state. Owns all OS-level handles, all background-task
/// `JoinHandle`s, the per-resource-class ID counters, and the delivery inbox.
/// The slot's `HostServices` impl is a thin handle into this struct (see
/// `SlotHostServices` below).
pub struct WidgetSlotHost {
    /// Per-resource-class ID allocators. One counter per class; bumped each
    /// time the matching `HostServices` method allocates a new handle. IDs
    /// are per-slot, so `WsId=5` in slot A and slot B are unrelated.
    next_ids: SlotIdCounters,

    fetch_handles:    HashMap<FetchId,             JoinHandle<FetchResult>>,
    ws_handles:       HashMap<WsId,                WsHandle>,
    socket_handles:   HashMap<SockId,              SocketHandle>,
    mdns_browses:     HashMap<MdnsBrowseId,        MdnsBrowseHandle>,
    mdns_registers:   HashMap<MdnsRegistrationId,  MdnsRegistrationHandle>,
    ssdp_handles:     HashMap<SsdpId,              SsdpHandle>,
    udp_handles:      HashMap<UdpId,               UdpHandle>,
    http_handles:     HashMap<HttpId,              HttpHandle>,
    audio_handles:    HashMap<AudioId,             AudioHandle>,
    led_handles:      HashMap<LedId,               LedHandle>,

    /// Per-slot inbox. Background tasks spawned for this widget are given a
    /// clone of `inbox_tx` at spawn time; it is the **only** route into this
    /// slot. The main loop drains `inbox_rx` after `tokio_drain_step` and
    /// invokes the appropriate `WasmWidgetRuntime::deliver_*` method on the
    /// runtime owned by this slot.
    inbox_rx: tokio::sync::mpsc::Receiver<HostEvent>,
    inbox_tx: tokio::sync::mpsc::Sender<HostEvent>,
    /// Sender for this widget's own Wayland surface — `play_sound`,
    /// `led_temporary`, etc. dispatch through the trait into this channel, the
    /// host's main loop drains it and emits the corresponding requests on this
    /// widget's `deck_widget_surface_v1`. The compositor disambiguates by the
    /// connection the request arrives on.
    wayland_out: WaylandSurfaceTx,
    shared: Rc<SharedHost>,
}

/// Payload carried over the slot's `inbox_tx` / `inbox_rx`. One variant per
/// delivery-bearing trait method; the main loop matches and dispatches into
/// the runtime via `WasmWidgetRuntime::deliver_*`.
pub enum HostEvent {
    FetchResponse(FetchId, FetchResult),
    FetchFailed(FetchId, HostError),
    WsFrame(WsId, WsMessage),
    WsClosed(WsId, WsCloseReason),
    SocketRead(SockId, Bytes),
    SocketClosed(SockId, SocketCloseReason),
    UdpDatagram(UdpId, SocketAddr, Bytes),
    MdnsBrowseEvent(MdnsBrowseId, MdnsRecordEvent),
    SsdpResponse(SsdpId, SsdpResponse),
    HttpRequest(HttpId, HttpRequest),
    AudioFinished(AudioId, AudioReason),
    LedFinished(LedId, LedReason),
}

/// `HostServices` impl carried by `HostState::host`. Holds `Rc<RefCell<WidgetSlotHost>>`
/// and `Rc<SharedHost>`; every method borrow_mut()s the slot for the duration of one
/// short, non-reentrant body (allocate ID, `tokio::spawn`, insert `JoinHandle`,
/// drop borrow). Reentrancy is impossible by construction: trait methods only
/// touch the slot's own maps and the tokio reactor, never call back into the
/// runtime or into other trait methods. Inbox draining and delivery happen in
/// a different phase of the main loop (after `tokio_drain_step`, before
/// `slot.render()`), so the runtime's `borrow_mut()` paths and the trait's
/// `borrow_mut()` paths never overlap in time.
pub struct SlotHostServices {
    slot: Rc<RefCell<WidgetSlotHost>>,
}
impl HostNetwork for SlotHostServices { /* … */ }
impl HostAudio   for SlotHostServices { /* … */ }
impl HostLed     for SlotHostServices { /* … */ }
```

The renderer itself is owned outside any `HostState` (on the host's stack during `slot.render()`) and reached only
through `renderer_ptr`. Tokio `JoinHandle`s registered in per-slot maps on `WidgetSlotHost` are aborted on slot drop,
freeing all per-widget resources; `HostState` itself contains no OS handles or tokio types, so its drop is
allocation-only.

ID / handle / pending-state invariant: a single per-slot, per-class counter (`WidgetSlotHost::next_ids`) allocates each
ID exactly once. The same ID keys two parallel maps on opposite sides of the seam — `*_handles` on `WidgetSlotHost`
(owns the OS resource and tokio `JoinHandle`, controls lifetime, aborted on slot drop) and `pending_*` on `HostState`
(owns the wasm-visible result state, populated by `WasmWidgetRuntime::deliver_*`, drained by the widget's poll imports).
The host removes from `*_handles` when the resource terminates; the runtime removes from `pending_*` when the widget
consumes the final delivery. Neither side allocates IDs.

`Rc` instead of `Arc`: in this Stage 7 reshape the host runs a `current_thread` Tokio runtime co-driven by the main poll
loop, so in practice only one thread touches `SharedHost` or `WidgetSlotHost`. Code spawning a Tokio task must clone
individual handles (`tokio::runtime::Handle`, `Arc<MdnsDaemon>`, `inbox_tx`) out of these structs rather than capturing
the `Rc<SharedHost>` or a slot-host reference itself; the `Rc` must never cross a thread boundary.

RNG state is per-widget to prevent correlated streams across widgets.

### Widget identity

The host runtime does not maintain its own widget-identity table. The compositor authoritatively binds each widget
instance to a Wayland connection (via `SO_PEERCRED` at `connect()` time). The host learns the widget's properties from
the initial Wayland configure batch (size, params JSON, settings). Audio, LED, sound, and any other commands that BMC
tags per-widget are emitted as requests on the widget's own `deck_widget_surface_v1`; the compositor identifies the
widget from the Wayland connection and forwards to BMC with the correct identity. The host neither knows nor needs to
know the numeric `instance_id` for routing.

For its own logs and metrics, the host uses the tuple `(peer_pid, wasm_basename)` — the PID it observed via
`SO_PEERCRED` on the Wayland connection plus the basename of the WASM module path from `Hello`. This is enough to
disambiguate widgets in journald output and to triage crashes against `ps`/`top` listings. The host neither knows nor
needs to know the numeric `instance_id` for routing.

If a future feature requires the host to correlate against `instance_id`, the protocol can be extended with an
`instance_id(u128)` event delivered as part of the configure batch.

## Isolation

One process now hosts N widgets. The design must guarantee that one widget cannot corrupt, observe, starve, or crash
another. Isolation is provided by a combination of wasmi's sandbox, per-widget data structures, and explicit caps.

### The five rules for per-widget async I/O

Every async I/O facility exposed to widgets (HTTP fetch, WebSocket, raw socket, mDNS browse, SSDP search, UDP broadcast,
HTTP listener, audio play, LED effect) follows the same five rules. Implementations that deviate are considered bugs.

The rules below describe the **initial-host model (Stages 5–6)**: each `WasmWidgetRuntime` owns its own per-resource ID
counters, maps, and `mpsc` channels — exactly as today. Stage 7 reshapes the implementation onto `WidgetSlotHost`

- `HostServices` (see § Host API isolation — Deferred); the rules themselves do not change, only where the structures
  live.

1. **Per-widget ID namespace.** Each `WasmWidgetRuntime` owns one counter per resource class on `HostState` (e.g.
   `next_ws_id`, `next_request_id`) and is the sole allocator of IDs for its widget. The same ID keys the runtime's own
   `websockets` / `sockets` / `mdns_browses` / `pending_*` maps. `WsId=5` in widget A's runtime and widget B's runtime
   are unrelated keys in unrelated maps. There is no global ID registry.
2. **Per-widget delivery channel captured at spawn.** Each background thread is spawned with a clone of an
   `mpsc::Sender` that resolves into the spawning runtime's `HostState` (and only that `HostState`). Today's runtime
   already constructs these senders per resource and stores their receivers in `HostState`; the host does not need to
   introduce any cross-slot routing. There is no global "look up widget by ID and deliver" path.
3. **No connection or engine state shared between widgets.** Widget A and widget B opening connections to the same URL
   get independent TCP sockets, TLS sessions, and in-flight request state because each runtime spawns its own `ureq` /
   `tungstenite` / `TcpStream` worker. The only shared resources host-side are the rendering singletons. Stage 7
   introduces a shared `reqwest::Client` and shared `mdns_sd::ServiceDaemon` that are stateless with respect to caller
   identity — they multiplex transport, not session.
4. **Per-widget concurrency cap.** Not enforced in the initial host: today's runtime allows unbounded
   thread-per-resource spawns. Stage 7 adds caps when imports route through `HostServices` (one error path per trait
   method). Until Stage 7 lands, the only bounds are OS thread / fd limits, and a misbehaving widget that opens 200
   WebSockets will spawn 200 OS threads before the kernel pushes back. Acceptable for ship-shape widgets; flagged as a
   hardening item.
5. **All per-widget tasks released on slot drop.** Slot teardown drops the `WasmWidgetRuntime`, which drops every
   `mpsc::Sender` and signals every `stop_tx` held in `HostState`. Worker threads exit on their next channel check.
   Threads blocked inside non-cancellable syscalls (`ureq.call()`, `tungstenite::accept`, `mdns-sd` socket reads) linger
   until their underlying I/O completes; they no longer hold any reference to slot state once the senders are dropped,
   so this is a thread-count tail, not a correctness issue. Stage 7's tokio `JoinHandle::abort()` removes even this
   tail.

Per-widget concurrency caps come in with Stage 7. The initial table below is the target for that stage; values are
starting guesses, to be revised against observed behaviour during implementation:

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

- The slot binds the staging FBO it currently owns and sets the viewport before calling into `runtime.render()`.

- The slot calls `renderer.begin_frame(w, h, 1.0)`, which resets femtovg transforms, paths, clips, and scissor.

- After `runtime.render()`, `renderer.flush()` drains pending draws.

- The blit pass explicitly sets its own program, VBO, and uniforms.

- GL state is normalized at the start of `slot.render()`, before any femtovg or blit call, to a fixed baseline so prior
  state cannot affect the new frame. The reset is:

  ```c
  glBindFramebuffer(GL_FRAMEBUFFER, slot_staging_fbo);
  glViewport(0, 0, slot_w, slot_h);
  glDisable(GL_SCISSOR_TEST);
  glDisable(GL_STENCIL_TEST);
  glDisable(GL_DEPTH_TEST);
  glDisable(GL_CULL_FACE);
  glDisable(GL_BLEND);                                  // femtovg re-enables as needed
  glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
  glDepthMask(GL_TRUE);
  glStencilMask(0xFF);
  glActiveTexture(GL_TEXTURE0);
  glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
  ```

  Any future host import that mutates raw GL state outside this set must restore it before returning, and the unit test
  below treats any leak as a failure.

A unit test verifies that two sequential `begin_frame` + arbitrary draws + `begin_frame` cycles produce identical output
for the second frame's draws — proving no femtovg-internal bleed.

### Font registration

Widgets may register their own fonts via host imports. Fonts live in the shared femtovg atlas; the host deduplicates
across slots by keying registration on the font bytes' content hash. The first widget to register a particular font pays
the upload cost (and an internal `FontId` is recorded against the hash); subsequent registrations of identical bytes by
any widget hand back the existing `FontId` without re-uploading.

**Eviction limitation.** femtovg 0.20.4 — the version pinned in this tree — exposes only `add_font*` methods on
`Canvas`; there is no `delete_font` / `remove_font` and the glyph atlas grows monotonically for the canvas's lifetime.
This means the host cannot reclaim atlas memory when the last widget referencing a font is dropped. The design accepts
this:

- The set of fonts shipped across all widgets is small and bounded by the firmware image; the steady-state cost is the
  sum of every unique font ever loaded since host start, not a per-widget cost.
- Deduplication still buys us "load once, not once per widget instance," which is the dominant saving.
- A refcount is still tracked per font for **observability and future eviction**: when femtovg gains a removal API (or
  if we patch our fork `femtovg-0.20.4-bdk445` to add one), eviction can be enabled without changing the registration
  path. Until then the refcount is informational.

If atlas growth becomes a problem in practice — e.g. a widget set that legitimately needs many distinct fonts in
rotation — the options are (a) patch the femtovg fork to support font removal, (b) restart the host on a heuristic
(font-count threshold), or (c) restrict the host import to a curated, pre-registered font set. None of these is required
at design time; the constraint is documented so the trade-off is visible.

### Panic safety

wasmi traps; it does not panic. A panic on the path through `slot.render()` therefore indicates a bug in *our own* host
import code (e.g., an `unwrap`/`expect` reached by a guest input combination we didn't anticipate). That bug should be
fixed when found — but in the meantime, a panic in one widget's render path must not take down the host and every
sibling widget with it. `slot.render()` is wrapped in `std::panic::catch_unwind` as defense in depth: the buggy slot is
dropped, the bug is logged loudly with the widget identifier (§ Widget identity), and the coordinator observes the thin
process exit and clears compositor state. Automatic respawn is future coordinator work; sibling widgets keep running.

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

Background workers are panic-isolated, by the same principle in both the initial host and the Stage 7 reshape:

- **Initial host (Stages 5–6).** Workers are `std::thread::spawn`-ed and not joined per tick. A panic unwinds the worker
  thread and drops its end of the `mpsc::Sender` it was writing into. The runtime sees the receiver close on its next
  drain in `poll_deliveries` and treats it as a normal "resource dead, close ID" event for that widget alone. No
  `JoinHandle` polling is needed because closed-channel semantics carry the same signal.
- **Stage 7.** Workers become tokio tasks. `JoinHandle` resolves with a `JoinError` of kind "panicked", which the
  deliver path converts into the same "resource dead, close ID" event for that widget alone.

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

These are head-of-line effects, not data leaks.

Both phases:

- **Render time.** Widgets render serialized on the single context; a slow widget delays others within a tick. Fuel cap
  bounds the delay.

Stages 5–6 only:

- **OS thread / fd budget.** Each in-flight HTTP fetch, WebSocket, raw socket, mDNS browse, etc. is its own OS thread on
  its own `mpsc` pair. A widget burst spawns many threads; the cost is paid by the whole host process via thread-table
  pressure and (for fetches) fresh connection setup with no shared pool. No enforced cap on the burst until Stage 7.
  This is the latency budget that motivates pulling Stage 7 in if Stage 6's thread-count measurement is alarming.

Stage 7 only:

- **HTTP connection-pool contention.** Widgets share a `reqwest::Client` pool; widget A burst can transiently delay
  widget B's connect. Per-widget concurrent-fetch cap bounds the burst.
- **Reactor scheduling.** Tokio's cooperative scheduler ensures fairness bounded by per-widget concurrency caps.

These cost predictable latency, not correctness or isolation.

## Compositor and client-library changes

Small and additive:

- `bmc-widget-protocol`: add `lifecycle_state` enum and `lifecycle` event. Regenerate Rust bindings. Re-export the
  generated `LifecycleState` enum at crate root so consumers do not need to spell the long
  `client::deck_widget_surface_v1::LifecycleState` path.
- `bmc-openwrt/src/compositor/widget_tracker.rs`: derive lifecycle state from existing scene/active/neighbor logic; emit
  `lifecycle` on change.
- `bmc-openwrt/src/compositor/protocol/dispatch.rs`: add `send_lifecycle()` helper on the per-widget surface dispatcher.
  PID-based identity logic (`SO_PEERCRED`, `set_widget_pid`, pending-connection buffer) is unchanged.
- `bmc-mock/src/mock_compositor.rs`: emit `lifecycle(visible)` once after the configure batch. `bmc-mock` is not
  currently used as a development entry point; revisit if that changes.
- `bmc-widget`: both surface dispatchers (`src/wayland.rs` and `src/surface/deck_widget.rs`) decode the `lifecycle`
  event into a `Lifecycle(LifecycleState)` variant on the public `WidgetEvent` / `DeckWidgetEvent` carriers and push it
  onto `pending_events` like any other event. The library is policy-free: it does not decide whether to act on the
  event. Consumers (today: legacy single-widget native binaries; tomorrow: the host runtime in this design) drain events
  and choose what to do — native widgets ignore the variant in their existing catch-all arm; the host runtime feeds it
  into the slot state machine described in § Render orchestration. `WEnum::Unknown(u32)` from a future protocol revision
  is dropped by the dispatcher with a warning, never surfaced as a typed event.

The compositor learns nothing new about widget topology.

## Error handling

| Failure                                                                                                      | Effect                                                                                                                                                                                                                                                |
| ------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| WASM widget panic (caught by `catch_unwind` around `slot.render()`) / fuel exhaustion / `RenderStatus::Dead` | Host drops the slot (closes the control socket); thin process sees EOF / `POLLHUP` on its read and exits. Coordinator cleanup observes child exit and clears compositor state; automatic respawn is future coordinator work. Other slots unaffected.  |
| Wayland disconnect for one widget                                                                            | Slot dropped (same path as above).                                                                                                                                                                                                                    |
| Control socket peer-closed by thin (host reads EOF / `POLLHUP`)                                              | Slot dropped.                                                                                                                                                                                                                                         |
| EGL context loss                                                                                             | Host exits non-zero; every thin wrapper sees EOF / `POLLHUP` on its control socket and exits; coordinator cleanup observes child exits and clears compositor state. Automatic respawn is future coordinator work.                                     |
| Host crashes / exits unexpectedly mid-flight                                                                 | Every thin wrapper sees EOF / `POLLHUP` on its control socket and exits cleanly. The coordinator clears pid/surface state from child-exit notifications. Automatic widget respawn/backoff is future coordinator work; no Stage 6 claim depends on it. |
| `gbm_bo_create` or `eglCreateImage` failure on a visibility transition                                       | Apply deterministic fallback from "Allocation failure behavior": preserve last valid frame when possible, mark slot `resource_blocked`, and retry on the bounded timer.                                                                               |
| Host fails to start                                                                                          | First thin wrapper exits non-zero after readiness-lock timeout or after final connect fails once the readiness lock is released. Future coordinator restart/backoff work may retry by spawning another thin.                                          |
| Bind race during host startup                                                                                | Loser exits; winner serves all clients.                                                                                                                                                                                                               |

## Migration / rollout

1. **Protocol + compositor change**, shippable on its own: add `lifecycle` event, wire it into `WidgetTracker`,
   regenerate bindings. Old widgets ignore the event; no behavior change.
2. **Host runtime + thin wrapper**: introduce `bmc-wasm-host` and `bmc-wasm-thin`. Update the coordinator's
   widget-wrapper script to exec `bmc-wasm-thin` instead of `widgets-wasm`. In the same change, delete the
   `widgets/wasm/` crate — there is no fallback path. Native non-WASM widgets (`widgets/flip-clock`,
   `widgets/digital-clock`) are unaffected; they keep using the 1:1 `bmc-widget` wrapper.

## Alternatives considered

### Alternative A — thin wrapper as lifetime witness (chosen)

This is what the rest of the document specifies: one OS process per widget that exists only to hold the Wayland
connection (so `SO_PEERCRED` continues to give the compositor a unique PID per widget) and a control socket (so the
host's read on that socket returns EOF / `POLLHUP` when the thin dies and the host drops the slot). The host runtime is
a separate, long-lived daemon, that is started by the first widget and quit by last widget exiting.

Trade-offs:

- **Kernel-enforced identity.** The compositor's existing `SO_PEERCRED`-based widget identity logic is reused verbatim.
  No new authentication mechanism. A widget cannot impersonate another even if the host itself is buggy, because the
  kernel sets peer credentials at `connect()` and they cannot be overwritten.
- **Coordinator's mental model unchanged.** Killing a widget is still killing its PID. The BMC coordinator's supervision
  tree, restart-backoff logic, and cleanup paths are unaffected.
- **Compositor changes are additive only** (new `lifecycle` event). No reshape of the per-widget identity model.
- **Residual memory cost.** Each thin process holds its own RSS — minimal (no EGL, no Renderer, no Tokio reactor) but
  non-zero, on the order of hundreds of KB per widget when measured as private anon + private file from
  `/proc/<pid>/smaps_rollup`.
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

- **Unit tests in `bmc-wasm-host`:** slot table operations, per-slot lifecycle apply (allocate-on-wake, free-on-dormant,
  no-op on render-target-preserving transitions), lifecycle state-machine reducer (pure function, no GL). Pool policy
  tests already live in `bmc-wasm-buffer-pool` and are not exercised by the host until the contingency in § Cross-widget
  pooling: deferred contingency is triggered.
- **Unit tests in `bmc-wasm-runtime`:** existing tests carry over after the `HostState` split; new tests cover the
  `with_renderer` guard including panic safety.
- **Integration test (host, headless):** spawn `bmc-wasm-host` against Mesa softpipe (or whichever software EGL the CI
  image carries), connect via a stub thin wrapper, load a small test WASM that draws a known pattern, read back the
  DMA-BUF, assert the pattern. Repeat with two widgets at once to verify non-interference.
- **End-to-end on device:** existing widget integration tests run unchanged.
- **Memory benchmark:** measure RSS (`smaps_rollup`, anon + private) and CMA before/after under a fixed scene with N
  WASM widgets. Target: the per-widget RSS delta drops by at least the 7 MB EGL init overhead and total RSS grows
  sub-linearly with widget count.

## Implementation stages

### Stage 0: `bmc-wasm-buffer-pool` crate — headless, generic

**Goal**: Land the pool's policy logic — entry state machine, `owner` tracking, acquire selection (affinity hit →
unaffinitized → allocate → steal → block), steal-by-priority ordering, byte accounting against the CMA ceiling,
connection teardown — as a standalone Rust crate with **no** Wayland or EGL dependency. The crate is generic over a
`Resource` type (the eventual `WidgetExportBuffer`) and a `ProxyBinding` trait (the eventual Wayland glue), so all later
stages plug into a fixed API.

**Public API** (see § Cross-widget pooling: deferred contingency for the intended use; the API is shipped against a
generic `ResourceFactory` / `ProxyBinding` so the eventual wire-up does not require crate-level changes):

```rust
pub enum LifecyclePriority { Dormant, Prepared, Entering, Visible }

pub trait ProxyBinding {
    type ProxyHandle;
    fn mint(&mut self, conn: ConnectionId, entry: EntryId) -> Self::ProxyHandle;
    fn destroy(&mut self, conn: ConnectionId, handle: Self::ProxyHandle);
}

pub struct BufferPool<R, B: ProxyBinding> { /* private */ }

impl<R, B: ProxyBinding> BufferPool<R, B> {
    pub fn acquire(&mut self, conn: ConnectionId, prio: LifecyclePriority, size: (u32, u32))
        -> Result<EntryId, AcquireError>;
    pub fn release_handle(&mut self, entry: EntryId);
    pub fn on_buffer_released(&mut self, entry: EntryId);
    pub fn teardown_connection(&mut self, conn: ConnectionId);
    pub fn set_priority(&mut self, conn: ConnectionId, prio: LifecyclePriority);
}
```

The ceiling is expressed in bytes; entry count is derived. `AcquireError::Blocked` carries enough information for the
daemon to wake the requester on the next `on_buffer_released`; the wake mechanism itself (e.g. `tokio::sync::Notify`)
lives on the daemon side, not the crate.

**Out of scope for this stage**: real DMA-BUF allocation, real `wl_buffer` proxies, real compositor lifecycle wiring,
async waits. All are mocked.

**Success criteria**: the crate builds standalone with `wayland_client` and `bmc-widget` **not** in its `Cargo.toml`;
the full algorithm passes unit tests against a `MockResource` and a `MockProxyBinding` that records calls.

**Tests** (unit, deterministic, no I/O):

- affinity hit returns the same entry across acquire/release cycles with no `mint` call;
- unaffinitized free entry triggers exactly one `mint`;
- allocation under the ceiling succeeds; over the ceiling falls through to steal;
- steal-from-lower-priority issues `destroy` on the previous owner's proxy then `mint` on the new owner;
- same-or-higher-priority affinity is never broken (`AcquireError::Blocked`);
- `on_buffer_released` after `release_handle` returns the entry to free with `owner` intact;
- `teardown_connection` calls `destroy` for every entry owned by that connection and returns their DMA-BUF resources to
  the unaffinitized free list;
- byte accounting is exact: every allocate adds, every free-and-truly-evict subtracts, never drifts.

**Status**: Landed; parked. The crate exists at `bmc-wasm-buffer-pool/` (commits `3eb672c7`…`9e698908`) and passes its
unit tests, but is **not** wired into the host. It is retained verbatim as the implementation backing the contingency in
§ Cross-widget pooling: deferred contingency. Later stages do not depend on this crate; if it turns out to be
permanently unused the crate may be deleted, but the cost of keeping a self-contained library around is negligible
compared to the cost of rebuilding the policy logic from scratch should the pool become necessary.

### Stage 1: `egl::EglState` split in `bmc-widget`

**Goal**: Decouple "owns EGL context" from "owns DMA-BUF + staging FBO" so the host can own one of the former and N of
the latter. **Success criteria**: existing `widgets/flip-clock`, `widgets/digital-clock` still build and run with no
behavior change; new `WidgetExportBuffer` type constructible from a borrowed `EglContext`. **Tests**: existing widget
tests pass; new unit test constructs two `WidgetExportBuffer`s against one `EglContext`. **Status**: Not Started

### Stage 2: `lifecycle` protocol event + compositor wiring

**Goal**: Ship the protocol extension and compositor-side state derivation without any host runtime changes. Old widgets
ignore the event. **Success criteria**: regenerated bindings build; `WidgetTracker` emits `lifecycle` on scene
transitions; debug logging on the widget side shows correct state sequence under manual scene cycling. **Tests**: unit
test for the `WidgetTracker → lifecycle_state` reducer; manual on-device check of the event stream. **Status**: Not
Started

### Stage 2.5: factor shared render scratch out of `widgets/wasm`

**Goal**: Separate the WASM render pipeline's GL resources into two ownership domains so that the eventual host can own
one set of "scratch" resources shared across N slots, while each slot owns only its DMA-BUF export pair. Today
`widgets/wasm/src/egl.rs` keeps the staging `WidgetExportBuffer` (color texture + stencil RBO + FBO) and `BlitResources`
(Y-flip program + VBO + attrib locations) as per-widget state; both are scratch — overwritten or unused between
`begin_frame` and `blit_to_export` — and trivially shareable under the single-threaded render model the host imposes (§
Renderer access from host functions). Concretely:

- Move `BlitResources` from `widgets/wasm/src/egl.rs` into `bmc-widget::egl` so it is reusable.
- Introduce `bmc-widget::egl::SharedRenderScratch { staging: WidgetExportBuffer, blit: BlitResources }`, borrow-based
  against `&EglContext` like `DoubleBufferState` already is. Construct once at a fixed `(max_width, max_height)` sized
  to the display max; widgets at smaller sizes set viewport on entry and the blit reads `(0,0,w,h)` of the staging
  texture.
- Make `bmc-widget::egl::DoubleBufferState` `pub` (the prerequisite already noted for Stage 5) so a future host can own
  one per slot against a borrowed `EglContext`.
- Refactor `widgets/wasm/src/egl.rs::EglState` to own one `EglContext`, one `SharedRenderScratch`, and one
  `DoubleBufferState`. Same external surface (`begin_frame` → femtovg → `blit_to_export` → `end_frame`), just dispatched
  through the new types. The standalone-widget process trivially has N=1 slot; the eventual host (Stage 5) relocates
  `EglContext` + `SharedRenderScratch` up into `SharedHost` and keeps `DoubleBufferState` per-slot with no further
  refactor.

Explicitly out of scope for this stage: no pool, no cross-slot `ExportBuffer` sharing, no rename of
`WidgetExportBuffer`, no behavior change for native widgets (flip-clock and the digital clock keep their direct-FBO
double-buffer pipeline exactly as today — neither uses the staging or blit). The Y-flip blit pass stays as-is;
eliminating it via compositor buffer-transform is captured separately in
`docs/devlogs/BDK-469/direct-dmabuf-render-deferred.md` and is not bundled here.

**Success criteria**: `widgets/wasm` renders correctly through the new types with no visible change against a Mesa
softpipe fixture and on-device; `bmc-widget::egl` exposes `SharedRenderScratch` and `DoubleBufferState` as public,
borrow-based types constructible against a single `EglContext`; `flip-clock` and other direct-FBO widgets build and
render unchanged; no per-frame GL allocation introduced; staging sized once at construction to a caller-chosen display
max and never resized; `SharedRenderScratch::begin_frame` re-establishes viewport and clears color + stencil on every
call, so no inter-slot GL state leakage is observable in the per-slot DMA-BUF outputs even when two slots of different
sizes render back-to-back through the same staging.

**Tests**: existing widget tests pass; new unit test constructs one `EglContext` + one `SharedRenderScratch` + two
`DoubleBufferState`s, renders a distinguishable solid colour through each by passing different export FBOs to the blit
entry point, and asserts the two exported DMA-BUFs hold the expected distinct pixels (mirrors the Stage 1 "two
`WidgetExportBuffer`s against one `EglContext`" test pattern, extended to verify cross-slot independence). Fixture
record/replay on at least one WASM example widget (`hello-widget` or `metronome`) continues to match baseline.

**Status**: Not Started

### Stage 3: `bmc-wasm-runtime` refactor — hoist the `Renderer` out

**Goal**: `WasmWidgetRuntime::new` constructs no `Renderer` and accepts no GL-related inputs. Concretely, the current
signature

```rust
pub unsafe fn new<F: FnMut(&str) -> *const c_void>(
    wasm_bytes: &[u8],
    load_fn: F,
    width: u32, height: u32,
    fbo_id: u32,
    config: RuntimeConfig,
) -> Result<Self>
```

loses `load_fn` (GL function-pointer loader — caller's concern, used only to build the renderer) and `fbo_id` (per-frame
render-target — caller's concern, set on the renderer when it binds its target FBO). `width` / `height` stay; they
describe widget dimensions, not GL. The `FemtoVgRenderer` is constructed and owned by the caller (testbed bin and
`widgets/wasm` rewired as part of Stage 3, host daemon in Stage 5) and reaches the runtime per-frame via
`with_renderer`, which installs `renderer_ptr` on `HostState` for the duration of the call (see § Renderer access from
host functions for the aliasing pattern and Miri test). The renderer field on `HostState`
(`host_state.renderer: FemtoVgRenderer`, today) is replaced by `renderer_ptr: Option<NonNull<Renderer>>`. The
`WasmWidgetRuntime::renderer()` accessor is removed; host import modules that need it use the `with_renderer` helper
from § Renderer access from host functions.

This stage is **strictly** the renderer refactor: per-widget I/O fields on `HostState` (`fetch_handles`, `websockets`,
`mdns_browses`, …) keep their current shape, the runtime's sync `ureq` / `tungstenite` / `mdns-sd` code paths are
unchanged, and no trait seam is introduced. The `HostServices` split is deferred to Stage 7, after the rendering
consolidation in Stages 4–6 ships; deferring it keeps each stage's diff small enough to review in isolation and lets the
rendering memory win land independently of the I/O refactor.

**Success criteria**: `WasmWidgetRuntime::new`'s signature contains no `load_fn`, no `fbo_id`, and no other GL type
(`glow::Context`, EGL handles, etc.); `WasmWidgetRuntime::renderer()` is gone; `HostState` no longer owns a
`FemtoVgRenderer`; standalone WASM widget continues to render correctly when driven by a caller-owned `Renderer` via
`with_renderer`; existing wasmi import behaviour is bit-identical to the pre-stage baseline (no I/O code touched);
`with_renderer` guard correctly installs and clears `renderer_ptr` including on panic.

**Tests**: existing widget tests pass unchanged; new unit test for the `with_renderer` install/clear/panic cycle
(`tests/with_renderer_aliasing.rs`); the same test is the target for the Miri aliasing job described under § Renderer
access from host functions. **Status**: Not Started

### Stage 4: testbed + `widgets/wasm` verification

**Goal**: Verify that `bmc-wasm-runtime/src/bin/testbed.rs` and `widgets/wasm/` behave on par with the pre-Stage-3
baseline now that `WasmWidgetRuntime` no longer owns the renderer. Both callers were rewired as part of Stage 3 — each
owns its `FemtoVgRenderer` and drives every render through the `with_renderer` guard — so this stage adds no production
code. It is a checkpoint to confirm the two caller-owned-renderer paths are correct end-to-end before the multi-widget
host work in Stage 5 builds on them.

**Success criteria**: the testbed renders every example widget across all four tile sizes with the existing multi-tile
preview, hot-reload via `notify`, interaction (touch, scroll), perf overlay, LED strips, and unified-fixture
record/replay all behaving identically to the pre-Stage-3 baseline; `widgets/wasm` boots on-device under the compositor,
paints, accepts touch input, and runs through fetch / WebSocket / mDNS / SSDP / UDP / HTTP / KV / params / LED paths
with no observable regression vs. the pre-Stage-3 baseline.

**Tests**: testbed manual sweep across the example widgets at each of the four tile sizes; hot-reload pass on
`pomodoro`; fixture record/replay roundtrip on `metronome`; on-device `widgets/wasm` deploy + smoke run of the same
widget set under the compositor. **Status**: Not Started

### Stage 5: `bmc-wasm-host` daemon — multi-widget, headless

**Goal**: Ship the daemon binary with a working main loop, control socket, fd handoff, lifecycle state machine, and
per-slot render-target management against the shared `EglContext`. The host hosts N `WasmWidgetRuntime`s as-is — each
runtime keeps its existing thread-per-resource sync I/O (`ureq`, `tungstenite`, `mdns_sd::ServiceDaemon`, raw sockets,
SSDP, UDP, HTTP listeners) and the `mpsc` channels rooted in its `HostState`. The host's main loop calls
`runtime.poll_deliveries()` on each slot per iteration to drain those receivers into `pending_*` state (§ Main loop).
**No tokio, no reqwest, no `HostServices` trait** in this stage; those land in Stage 7. `bmc-wasm-host` does **not** add
`tokio` or `reqwest` to the workspace.

`SharedHost` in Stage 5 owns the rendering singletons only — `EglContext`, `glow::Context`, `Renderer`,
`SharedRenderScratch`, `BlitShader`, font cache — not Tokio, not `reqwest::Client`, not a shared mDNS daemon.

Each slot owns its `DoubleBufferState` plus an optional `WidgetExportBuffer` (allocated against `shared.egl` on wake-up,
destroyed on dormancy); the slot mints its own `wl_buffer` proxies on its Wayland connection via
`zwp_linux_dmabuf_v1.create_params` + `create_immed`, paired one-to-one with its export buffers, and registers
`wl_buffer.release` listeners that drive its local ping-pong bookkeeping. There is no shared pool, no priority, no
steal. Slot drop tears down both buffers and proxies. Perf overlay rendering is host-singleton: each slot reports
per-frame timings to `SharedHost`, the host aggregates across active slots and draws one overlay per scene (not per
slot) using the shared `Renderer`. There is no intermediate single-widget configuration.

**Per-slot teardown requirement.** Standalone today, a widget process exit lets the OS reap worker threads blocked in
`ureq.call()` / `tungstenite::accept` / `TcpStream::read` / `mdns-sd` recv. In the host, the OS cannot help: the slot
must drop the `WasmWidgetRuntime`, which in turn must drop every `mpsc::Sender` in `HostState` and signal every
`stop_tx` so worker threads exit on their next channel check. Threads stuck in non-cancellable syscalls linger until
their I/O resolves but no longer reference slot state; that's a thread-count tail, not a leak. If `WasmWidgetRuntime`
today relies on process exit for cleanup, this stage adds the explicit shutdown path.

**`mdns_sd::ServiceDaemon` multi-instance check.** Today each widget process has its own daemon binding mDNS multicast
(5353). Stage 5 includes a smoke test running two runtimes that each `mdns_browse` the same service in one process and
asserting both receive announcements. If `mdns-sd` cannot multi-bind 5353 in a single process, Stage 5 narrowly
introduces a shared `Arc<ServiceDaemon>` on `SharedHost` (a precursor to 7; the rest of the I/O model stays
per-runtime).

**Prerequisite**: `DoubleBufferState` in `bmc-widget::egl` must be made `pub` so the host can construct it against a
borrowed `EglContext` (a one-line visibility change to a struct that already has all the methods the host needs).

**Success criteria** (revised — see `docs/devlogs/BDK-469/stage-5-multi-widget-host-spec.md`): the `bmc-wasm-host`
daemon binary builds and runs; the control socket accepts a `Hello` with a Wayland fd via `SCM_RIGHTS` and returns
`Ack::Ok` on success / `Ack::Err` on failure; the slot lifecycle state machine implements the simplified
`{Entering, Visible, Leaving}`-only allocate/render set with deterministic transitions and a bounded retry timer on
allocation failure; per-runtime shutdown unblocks workers within a bounded window measured by the multi-runtime teardown
test below; mDNS multi-instance test passes (per-runtime daemons coexist) or the shared-daemon fallback lands as part of
this stage. End-to-end on-device validation with multiple widgets is **deferred to Stage 6** — without `bmc-wasm-thin`
the compositor's `SO_PEERCRED` identity binding has nothing to bind to and a synthetic multi-widget harness cannot reach
it.

**Tests**: control-socket handshake test (socketpair fd → `Hello` → `Ack::Err`, proves the wire format and `SCM_RIGHTS`
end-to-end without a compositor); lifecycle state-machine unit test against a mock render-target factory covering the
full 5×5 matrix plus the `resource_blocked` retry path; **multi-runtime teardown test** in `bmc-wasm-runtime`: two
runtimes both performing fetch / WS / mDNS, drop one mid-flight, assert the other keeps working and the dropped
runtime's `HostState`'s `stop_tx` / `Sender` clones are disconnected within a bounded window; **mDNS coexistence test**:
two runtimes browsing the same service, both receive announcements (or, if loopback multicast cannot be made to work,
the shared-daemon fallback is introduced and the test pivots to assert against the shared daemon). The
headless-compositor two-widget cycling test and the connection-teardown buffer-free test from the original plan are
dropped — they require a compositor that cannot be stood up convincingly without `bmc-wasm-thin`; both are covered
end-to-end as part of Stage 6 on-device acceptance. **Status**: Not Started

### Stage 6: `bmc-wasm-thin` wrapper + coordinator integration

**Goal**: Make `bmc-wasm-thin` the default (and only) path for WASM widgets and delete `widgets/wasm/`. Coordinator
unchanged. **Success criteria**: device boot brings up multiple WASM widgets via the host runtime; killing a thin
process tears down its widget without affecting siblings; a host crash makes every thin exit and clears compositor state
without claiming automatic respawn; measured RSS and CMA improvement under a scene with N≥4 WASM widgets vs. baseline;
**thread-count measurement under a representative scene** stays within an acceptable budget (target: dominated by
render-loop threads + a small constant per active runtime, not the OS thread limit). If the measurement is alarming —
i.e., bursts during normal scene cycling push the process toward kernel thread limits, RLIMIT_NPROC, or visible
scheduling pressure — Stage 7 is pulled forward before further widget onboarding. **Tests**: on-device scene cycle;
thin-process kill test; host-crash test (SIGKILL the host, confirm every thin exits and the coordinator clears
pid/surface state, no automatic respawn expected); memory benchmark (`smaps_rollup`); thread-count snapshot under a
four-widget scene with active fetches and a WebSocket; soak test cycling visibility for 10 minutes. **Status**: Not
Started

### Stage 7: `bmc-wasm-runtime` I/O seam — introduce `HostServices` and consolidate I/O onto a shared reactor

**Trigger**: ships either (a) opportunistically once Stages 5–6 are deployed and stable, to claim the thread-count and
per-widget engine-state savings, or (b) under pressure if the Stage 6 thread-count measurement is alarming, if mDNS
multi-instance falls back to a shared daemon and the rest of the I/O wants to follow, or if per-widget concurrency caps
become required for hardening.

**Goal**: Define `HostNetwork` + `HostAudio` + `HostLed` (plus the marker super-trait `HostServices`) in
`bmc-wasm-runtime`, matching the existing `runtime/imports/*.rs` module layout. Replace `HostState`'s per-resource
`mpsc` channels and worker handles with `pending_*` maps for deliverable state plus an `Rc<dyn HostServices>` for
outbound calls (§ Host API isolation — Deferred design). Rewrite every wasmi host import to dispatch through the trait
instead of touching `ureq` / `tungstenite` / `mdns-sd` directly. The runtime crate's top-level `[dependencies]`
**drops** `ureq`, `tungstenite`, and `mdns-sd`, and does **not** gain `tokio` or `reqwest`; net library-dependency
change is a removal. Those three crates move under `[features].testbed` so the library cone stays clean while the
testbed continues to provide a minimal sync `HostServices` impl built on `ureq` + a small thread pool, `tungstenite` on
a worker thread, and `mdns-sd`.

In `bmc-wasm-host`, this stage introduces the host-side machinery described in § Host API isolation — Deferred and §
Per-runtime I/O integration: a Tokio `current_thread` runtime co-driven by the main poll loop via a bounded
`tokio_drain_step`; `SharedHost` extensions for `tokio::runtime::Handle`, shared `reqwest::Client`, and
`Arc<MdnsDaemon>`; per-slot `WidgetSlotHost` carrying tokio `JoinHandle` maps, the inbox channel (`tokio::sync::mpsc`),
the per-slot ID counters, and the Wayland surface sender; the `SlotHostServices` impl plumbing the trait through to the
slot's maps. Add per-widget concurrency caps from § The five rules at the trait boundary (one error path per method).
Wire `HostEvent` delivery from inbox drain back into `WasmWidgetRuntime::deliver_*`.

**Success criteria**: `bmc-wasm-runtime`'s library cone builds with no `tokio` / `reqwest` / async-runtime / `ureq` /
`tungstenite` / `mdns-sd` dependency (those crates appear only under `[features].testbed`); the host runs N widgets with
one shared reactor, one `reqwest::Client`, one shared `mdns_sd::ServiceDaemon`; per-widget thread count collapses to
bounded reactor-task count instead of thread-per-resource; per-widget caps return `HostError` at the trait boundary on
overflow; previous Stage 5/6 behavioural and memory acceptance criteria still hold; all wasmi imports route through
`HostServices`.

**Tests**: existing wasmi import tests, rewritten to use a stub `HostServices` impl that records calls; trait-object
dispatch verified by a smoke test that swaps in a second `HostServices` impl returning canned `HostError`s; rerun the
Stage 5 multi-runtime teardown test and the mDNS coexistence test against the consolidated reactor; thread-count
snapshot under the Stage 6 four-widget scene shows the expected drop versus the Stage 6 baseline.

**Note**: the cross-slot font/glyph/image-cache deduplication mentioned elsewhere is a Stage 5 consequence of one
`Renderer` serving N slots, **not** a Stage 7 deliverable — nothing in this stage changes the cache shape.

**`FontCache` is wired in Stage 7, not Stage 5.** Stage 5 lands the `FontCache` *field* on `SharedHost` and a
SHA-256-keyed dedup map as scaffolding (so the structure doesn't have to be re-touched), but the `register_font` host
import that drives it is part of the `HostServices` host-API consolidation. Stage 5 ships `FontCache`
`#[allow(dead_code)]` and Stage 7 introduces:

- A `register_font` import on `HostNetwork` (or a new `HostFonts` sub-trait if scope warrants), called by guests during
  their `__bmc_setup` to materialize font assets.
- Wiring from the import into `FontCache::register` on the host singleton; the returned `FontId` is what the guest
  passes to femtovg via the renderer pointer.
- Validation that the SHA-256 key shape survives whatever the import's payload turns out to be (raw bytes, asset-bundle
  index, or a path) — if the implementation forces a different key, Stage 7 changes `FontCache`'s map type. The Stage 5
  scaffold is intentionally narrow to keep this open.

If by the time Stage 7 lands `FontCache` is still untouched and the cache shape needs revisiting, prefer deleting the
Stage-5 scaffold and adding a fresh field over keeping a stale type.

**Status**: Not Started

### Stage 8 (contingent): wire `bmc-wasm-buffer-pool` into the host

**Trigger**: on-device measurements during Stage 6 (or later) showing one of the conditions in § Cross-widget pooling:
deferred contingency — i.e. visible hitch from per-wake EGLImage import during scene drags, CMA fragmentation causing
allocation slowdown or failure after extended on/off cycling, or aggregate CMA pressure under widget growth.

**Goal**: Move buffer ownership from per-slot to a single `BufferPool` on `SharedHost`. Slots hold `EntryId`s instead of
owned `ExportBuffer`s; `apply_lifecycle` calls `Pool::acquire` / `Pool::release_handle` instead of allocating /
destroying directly; the main loop runs the two-pass release-then-acquire ordering described in § Batch ordering on
scene swaps (which the compositor already emits, so this is host-side only); `wl_buffer.release` listeners forward to
`Pool::on_buffer_released`; connection teardown sweeps the pool. Add `ResourceFactory::destroy(&mut self, R)` to the
crate so the pool can free CMA on `Drop`.

**Success criteria**: the failing measurement from the trigger condition recovers (e.g. seam-hitch disappears, CMA
fragmentation stabilizes, ceiling-bound aggregate fits); existing tests still pass; the pool's policy unit tests
(already in `bmc-wasm-buffer-pool`) continue to pass without change.

**Tests**: re-run the trigger measurement and verify the metric meets target; headless integration test (Mesa softpipe)
with a deliberately constrained pool ceiling that forces a steal — assert the new owner gets a freshly-minted
`wl_buffer` ObjectId and the old owner's proxy was destroyed; soak test cycling visibility for 10 minutes with no
allocation events past the warmup window.

**Status**: Not Started; do not begin until a trigger condition is documented.

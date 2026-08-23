# WASM Host Render Loop

This document describes how `bmc-wasm-host` drives multiple WASM widget slots in one process: which slots own render
targets, which slots can render, how frame callbacks are scheduled, and how teardown works.

For process startup, fd passing, and thin/daemon lifetimes, see [`process-model.md`](process-model.md).

## Main Loop Shape

`bmc-wasm-host` has one shared renderer and a table of `WidgetSlot`s. The loop in `bmc-wasm-host/src/main_loop.rs` is
single-threaded and uses `poll(2)` over:

- the host listener fd;
- each slot's Wayland fd;
- each slot's thin control socket fd.

Each iteration does this:

1. Compute a `poll(2)` timeout from all slots and the host lifetime grace period.
2. Poll the listener, Wayland fds, and control sockets.
3. Accept and load new slots when the listener is readable.
4. For every existing slot:
   - dispatch Wayland events;
   - check the control socket lifetime witness;
   - apply lifecycle transitions and allocate or release render targets;
   - poll runtime deliveries.
5. For every remaining slot, render only if lifecycle and timing gates allow it.
6. Tear down slots that disconnected, lost Wayland, or returned a fatal render status.
7. Exit after the last slot is gone and the 100 ms host grace period expires.

There is no per-widget render thread. Heavy GPU state is shared in `SharedHost`:

- `EglContext`;
- `SharedRenderScratch`;
- `FontCache`.

Each slot owns its own `WasmWidgetRuntime`, Wayland surface client, control socket, lifecycle state machine, and
optional render target. Aliasing invariant: SharedHost does NOT own the Renderer. The renderer is owned by
main_loop::run and passed into per-slot work as `renderer_ptr` on each loop pass.

## Lifecycle States

The compositor sends `deck_widget_surface_v1.lifecycle` events. The host maps those protocol values to
`bmc-wasm-host::lifecycle::LifecycleState`:

| State      | Has render target | May render        | Host-requested frame loop | Meaning                                                            |
| ---------- | ----------------- | ----------------- | ------------------------- | ------------------------------------------------------------------ |
| `Dormant`  | No                | No                | No                        | Off-screen. Keeps runtime state but owns no GPU export buffers.    |
| `Prepared` | Yes               | Warm-up or forced | No                        | Immediate neighbour. Pre-renders one frame, then holds one buffer. |
| `Entering` | Yes               | Warm-up or forced | No                        | Drag target entering the screen.                                   |
| `Visible`  | Yes               | Yes               | Yes                       | Active on-screen widget.                                           |
| `Leaving`  | Yes               | Warm-up or forced | No                        | Current widget leaving during a drag.                              |

The key host predicates are:

- `has_render_target(state)` is true for `Prepared`, `Entering`, `Visible`, and `Leaving`;
- `should_render(state)` matches `has_render_target(state)`;
- `frame_callback_enabled(state)` is true only for `Visible`.

This split is deliberate. `Prepared` is the pre-render state: the compositor marks immediate neighbours `Prepared` so
the host allocates a render target and renders a single frame ahead of a possible drag, making the later transition into
`Entering` instant. The pre-render is triggered on entry into `Prepared` (the dirty surface raised by the lifecycle
event); the transition from `Prepared` to `Entering` does not itself trigger a new render, and `Prepared` runs no guest
animation loop. Once a frame has been committed for the current target, later dirty events are held while the slot is
`Prepared`, `Entering`, or `Leaving`. The dirty flag remains set and is rendered when the slot becomes `Visible`.
`transition_incoming` is the exception: it explicitly forces one dirty pre-transition frame through the gate so an
automatic scene transition starts with current content. `Leaving` keeps its render target so the compositor can still
animate the scene drag by repainting and moving the widget's last submitted buffer, but it no longer drives runtime
animation frames. The compositor only completes frame callbacks that the client requested on a commit; the current host
does not request those callbacks when submitting a `Prepared`, `Entering`, or `Leaving` buffer.

## Lifecycle Application

Each slot starts as `Dormant` with `render_target = None`.

When a lifecycle event arrives, the slot updates its target state. If the new target acquires a render target the
previous target did not have (entering the render-set from `Dormant`) and the state actually changes, the slot marks its
Wayland surface dirty so the host renders after applying the transition. This is what produces the single `Prepared`
pre-render; transitions that stay inside the render-set (for example `Prepared` to `Entering`) do not mark the surface
dirty on their own.

`LifecycleStateMachine::apply` is responsible for target ownership:

- moving from `Dormant` into the render-set (`Prepared`, `Entering`, `Visible`, `Leaving`) allocates a `RenderTarget`;
- moving from a render-set state back to `Dormant` releases the `RenderTarget`;
- transitions between render-set states keep the existing target;
- acquiring a target clears `rendered_since_acquire`, re-arming its one unconditional warm-up frame;
- allocation failures set the slot as blocked and retry after 1 second;
- a later transition back to `Dormant` clears a blocked allocation.

The production render target is an `EglRenderTarget`:

- a per-slot `DoubleBufferState`;
- up to two exported DMA-BUF buffers;
- up to two pre-created `wl_buffer` proxies for the slot's Wayland surface.

A freshly allocated target double-buffers (two export buffers). While the slot is `Prepared`,
`LifecycleStateMachine::apply` calls `compact_for_prepared` every loop iteration; it releases the spare, not-in-flight
buffer as soon as one exists — right after the `Dormant` to `Prepared` allocation, before the slot's first prepared
render, or the iteration after a compositor release frees the spare. A steady-state `Prepared` slot therefore holds a
single buffer; the second buffer is reallocated only if the slot renders again (a forced pre-transition frame, or once
`Visible`).

Buffer destruction is deferred until the compositor releases the buffer. `apply` never frees a buffer the compositor may
still be reading: `destroy_released_slots` frees only slots the compositor has already released (tracked by
`bmc_widget::egl::SlotReleaseState`), and any still-held slots are parked in `retired_render_targets` and reclaimed
later once the release arrives. Releasing the target this way returns the per-slot CMA-backed render memory once the
widget no longer needs it.

## Render Eligibility

The host renders a slot only when `WidgetSlot::needs_render(now)` returns true.

That requires all of these conditions:

1. The slot lifecycle is renderable: `Prepared`, `Entering`, `Visible`, or `Leaving`.
2. The slot is not blocked on render-target allocation retry.
3. Either the Wayland surface is dirty and passes the off-screen gate, or a runtime frame deadline is due.
4. Runtime frame deadlines count only when the host-requested frame loop is enabled, which means `Visible`.
5. The per-slot minimum inter-frame interval has elapsed.

The minimum inter-frame interval is 8 ms. It caps a widget that continuously asks for immediate frames at roughly 120
fps.

Lifecycle-owned surface dirtiness paints a fresh render target and services `transition_incoming`. After the warm-up
frame, dirty state is held in `Prepared`, `Entering`, and `Leaving` and released when the slot becomes `Visible`.

Network, parameter, system-setting, credential, and touch deliveries invoke guest hooks without dirtying the surface.
Only `request_frame()`/`request_frame_after()` from the guest enters the runtime scheduler, which renders while
`Visible`.

## Rendering A Frame

When a slot is eligible, `WidgetSlot::render` does the GPU work:

01. Clear the surface dirty flag.
02. Ensure the slot has a current export buffer in its double-buffer state.
03. Bind the shared scratch staging FBO for this slot's size.
04. Normalize GL state before handing control to the runtime renderer.
05. Begin a femtovg frame on the shared renderer.
06. Call `runtime.with_renderer(..., |rt| rt.render(delta_ms))`.
07. Flush femtovg.
08. Blit from shared scratch into the slot's current export FBO.
09. Finish GL work.
10. Export the current buffer metadata and swap to the next buffer slot.
11. Submit the matching cached `wl_buffer` to the compositor.
12. Flush the Wayland connection.
13. Schedule the next runtime frame if the guest requested one.

The host calls `runtime.set_time(...)` immediately before rendering. `delta_ms` is derived from the slot's previous
render timestamp, and the slot also exposes a monotonic millisecond clock to the runtime.

If rendering returns `RenderStatus::Dead`, if the render closure errors, or if it panics, the slot is torn down. If the
EGL context is lost, the whole host exits with a fatal error.

## Poll Timeout Sources

`compute_poll_timeout` folds each slot into a single timeout for `poll(2)`.

The timeout can be shortened by:

- lifecycle allocation retry deadlines;
- dirty renderable surfaces that pass the off-screen gate;
- due runtime frame deadlines for `Visible` slots;
- future runtime frame delays for `Visible` slots;
- the 8 ms minimum inter-frame remainder;
- pending runtime I/O;
- the host's 100 ms post-disconnect grace period.

If a renderable slot has an effective dirty flag or an immediate runtime frame due and the inter-frame floor has
elapsed, the timeout is `0`, so the host loops without sleeping. The effective flag masks held-back dirtiness in
`Prepared`, `Entering`, and `Leaving`, preventing those slots from spinning the poll loop. Runtime frame deadlines are
also ignored until the slot becomes `Visible`.

Current implementation detail: pending runtime I/O contributes a 100 ms timeout if any slot reports
`runtime.has_pending_io()`. This is not lifecycle-gated today, so a dormant slot with pending background work can still
keep the host waking at that cadence.

## Runtime Deliveries

After Wayland/control dispatch and lifecycle application, the loop stages delivery readiness for every slot. CPU-only
work can be staged without acquiring the renderer. If pending lifecycle or callback work can access the renderer, the
host enters the cross-process GPU lock, polls the staged renderer deliveries, and waits for GPU completion before
leaving the scope. An idle slot, including a dormant slot with suspended reservations, does not acquire the lock or
issue a fence merely because it exists.

Deliveries include completed fetches and image decodes, sockets, WebSocket and discovery events, delayed work, and
lifecycle hooks. A callback may mutate guest state and request a frame. Lifecycle gating still decides whether that
request can become an actual render. Renderer asset suspension and restoration are described in
[`renderer-assets.md`](renderer-assets.md).

## Compositor Lifecycle Emission

The compositor derives lifecycle state from `WidgetTracker`:

- active scene widgets are `Visible` while idle;
- active scene widgets become `Leaving` during a drag;
- immediate neighbours are `Prepared`;
- the drag-direction neighbour becomes `Entering`;
- other visible widgets in the scene-cycling list are `Dormant`.

Lifecycle transitions are emitted in release/acquire batches:

1. Send transitions into `Dormant`.
2. Flush affected clients.
3. Send transitions out of `Dormant` or between non-`Dormant` states.
4. Flush affected clients.

This ordering lets hosts release render targets on `Dormant` before other slots allocate new ones. The current WASM host
has per-slot render targets rather than a cross-widget pool; `Prepared` now owns a single pre-rendered buffer, so the
release-before-acquire ordering still bounds peak buffer memory across a transition.

On first connection, the compositor sends an initial lifecycle event after the configure batch. If the current tracker
state would be `Entering`, the compositor clamps the initial event to `Prepared`. If it would be `Leaving`, it clamps to
`Visible`. Transitional states only make sense as deltas from a prior idle state a brand-new client cannot have seen.

## Teardown

A slot is removed when any of these happen:

- the thin control socket closes or hangs up;
- the Wayland connection fails or disconnects;
- the runtime render result is fatal;
- EGL context loss forces host shutdown.

Slot shutdown drops the runtime first, then destroys any render target, then drops the Wayland surface client and the
control socket.

After the table becomes empty, `HostLifetime` records the disconnect time. The main loop keeps running for 100 ms, then
returns cleanly if no new slot arrived.

## Code Map

- `bmc-wasm-host/src/main_loop.rs` - polling, timeout folding, slot iteration, host lifetime.
- `bmc-wasm-host/src/slot.rs` - per-slot dispatch, render gating, frame rendering, shutdown.
- `bmc-wasm-host/src/lifecycle.rs` - lifecycle state machine and allocation retry behavior.
- `bmc-wasm-host/src/render_target.rs` - per-slot render target allocation and destruction.
- `bmc-wasm-host/src/host.rs` - shared EGL, scratch FBO, and font cache.
- `bmc-widget-protocol/protocol/deck-widget.xml` - lifecycle protocol contract.
- `bmc-openwrt/src/compositor/widget_tracker.rs` - compositor-side lifecycle derivation.
- `bmc-openwrt/src/compositor/lifecycle_emitter.rs` - release/acquire transition batching.
- `bmc-openwrt/src/compositor/egl_compositor.rs` - lifecycle emission and client flush ordering.

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
- `FemtoVgRenderer`;
- `FontCache`.

Each slot owns its own `WasmWidgetRuntime`, Wayland surface client, control socket, lifecycle state machine, and
optional render target.

## Lifecycle States

The compositor sends `deck_widget_surface_v1.lifecycle` events. The host maps those protocol values to
`bmc-wasm-host::lifecycle::LifecycleState`:

| State      | Has render target | May render      | Continuous frame callbacks | Meaning                                                                 |
| ---------- | ----------------- | --------------- | -------------------------- | ----------------------------------------------------------------------- |
| `Dormant`  | No                | No              | No                         | Off-screen. Keeps runtime state but owns no GPU export buffers.         |
| `Prepared` | Yes               | Yes, when dirty | No                         | Immediate neighbour. Can pre-render one fresh frame.                    |
| `Entering` | Yes               | Yes, when dirty | No                         | Drag target entering the screen. Host avoids continuous animation here. |
| `Visible`  | Yes               | Yes             | Yes                        | Active on-screen widget.                                                |
| `Leaving`  | Yes               | Yes             | Yes                        | Current widget leaving during a drag.                                   |

The key host predicates are:

- `has_render_target(state)` is true for `Prepared`, `Entering`, `Visible`, and `Leaving`;
- `should_render(state)` currently matches `has_render_target(state)`;
- `frame_callback_enabled(state)` is true only for `Visible` and `Leaving`.

This split is deliberate. `Prepared` and `Entering` may need a fresh buffer, but they must not spin an animation loop
just because the guest runtime asks for the next frame.

## Lifecycle Application

Each slot starts as `Dormant` with `render_target = None`.

When a lifecycle event arrives, the slot updates its target state. If the new target has a render target and differs
from the previous target, the slot marks its Wayland surface dirty so the host will render after applying the
transition.

`LifecycleStateMachine::apply` is responsible for target ownership:

- moving from `Dormant` to a target-owning state allocates a `RenderTarget`;
- moving from a target-owning state to `Dormant` destroys the `RenderTarget`;
- transitions between target-owning states keep the existing target;
- allocation failures set the slot as blocked and retry after 1 second;
- a later transition back to `Dormant` clears a blocked allocation.

The production render target is an `EglRenderTarget`:

- a per-slot `DoubleBufferState`;
- two exported DMA-BUF buffers;
- two pre-created `wl_buffer` proxies for the slot's Wayland surface.

Destroying the target destroys both export buffers and both `wl_buffer`s. This releases the per-slot CMA-backed render
memory while the widget is dormant.

## Render Eligibility

The host renders a slot only when `WidgetSlot::needs_render(now)` returns true.

That requires all of these conditions:

1. The slot lifecycle is renderable: `Prepared`, `Entering`, `Visible`, or `Leaving`.
2. The slot is not blocked on render-target allocation retry.
3. Either the Wayland surface is dirty or a runtime frame deadline is due.
4. Runtime frame deadlines count only when frame callbacks are enabled, which means `Visible` or `Leaving`.
5. The per-slot minimum inter-frame interval has elapsed.

The minimum inter-frame interval is 8 ms. It caps a widget that continuously asks for immediate frames at roughly 120
fps.

Dirty surface renders come from lifecycle changes, param updates, touch input, and other host-side events that call
`mark_needs_render()`. Runtime animation renders come from `WasmWidgetRuntime::wants_next_frame()` and
`next_frame_delay()` after a previous render.

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
- dirty renderable surfaces;
- due runtime frame deadlines for `Visible` or `Leaving` slots;
- future runtime frame delays for `Visible` or `Leaving` slots;
- the 8 ms minimum inter-frame remainder;
- pending runtime I/O;
- the host's 100 ms post-disconnect grace period.

If a renderable slot is dirty or has an immediate runtime frame due and the inter-frame floor has elapsed, the timeout
is `0`, so the host loops without sleeping.

Current implementation detail: pending runtime I/O contributes a 100 ms timeout if any slot reports
`runtime.has_pending_io()`. This is not lifecycle-gated today, so a dormant slot with pending background work can still
keep the host waking at that cadence.

## Runtime Deliveries

The current loop calls `runtime.poll_deliveries_with_renderer(renderer_ptr)` for every slot on every loop pass, after
Wayland/control dispatch and lifecycle application.

This delivery polling can process host work such as completed fetches, sockets, WebSocket events, discovery events, and
delayed work. A delivery callback may mutate guest state and request a frame. Lifecycle gating still decides whether
that frame request can become an actual render.

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
3. Send transitions out of `Dormant` or between target-owning states.
4. Flush affected clients.

This ordering lets hosts release render targets before other slots allocate new ones. The current host has per-slot
render targets rather than a cross-widget pool, but the ordering still preserves the intended memory-pressure behavior.

On first connection, the compositor sends an initial lifecycle event after the configure batch. Initial `Entering` and
`Leaving` are clamped to `Visible` because transitional states only make sense as deltas from a prior state the widget
has already seen.

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
- `bmc-wasm-host/src/host.rs` - shared EGL, scratch FBO, renderer, and font cache.
- `bmc-widget-protocol/protocol/deck-widget-v1.xml` - lifecycle protocol contract.
- `bmc-openwrt/src/compositor/widget_tracker.rs` - compositor-side lifecycle derivation.
- `bmc-openwrt/src/compositor/lifecycle_emitter.rs` - release/acquire transition batching.
- `bmc-openwrt/src/compositor/egl_compositor.rs` - lifecycle emission and client flush ordering.

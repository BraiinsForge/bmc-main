# Widget Lifecycle via `deck_widget_v1`

This document describes the widget lifecycle event sent by the compositor over the `deck_widget_v1` Wayland protocol. It
focuses on where lifecycle state comes from, when the compositor emits it, and what contract widget clients can rely on.

For widget geometry, params, settings, and the initial configure batch, see
[`widget-runtime-configuration.md`](widget-runtime-configuration.md). For how the WASM host uses lifecycle events to
allocate buffers and gate rendering, see [`wasm-host/render-loop.md`](wasm-host/render-loop.md).

## Purpose

Lifecycle state tells a widget how close it is to the currently displayed scene:

- completely off-screen;
- designated as an immediate neighbour, with optional client-side prewarming;
- entering or leaving during a scene drag;
- active on screen.

The compositor sends this as a `deck_widget_surface_v1.lifecycle(state)` event. The event is compositor-to-widget only;
widgets do not request lifecycle changes directly.

The primary consumer today is the WASM host. It uses lifecycle state to hold render targets while widgets are
`Prepared`, `Entering`, `Visible`, or `Leaving` (pre-rendering a single frame in `Prepared`), and to animate only the
`Visible` widget. Native widgets may also consume the event through `WidgetEvent::Lifecycle`, but widgets that do not
care about lifecycle can ignore it.

## Protocol States

The protocol enum lives in `bmc-widget-protocol/protocol/deck-widget-v1.xml`.

| Protocol state | Value | Meaning                                                                     |
| -------------- | ----: | --------------------------------------------------------------------------- |
| `dormant`      |     0 | Widget is off-screen and should not need a render target.                   |
| `prepared`     |     1 | Widget is an immediate scene-cycling neighbour and may be pre-rendered.     |
| `entering`     |     2 | Widget is the drag-direction neighbour currently moving onto the screen.    |
| `visible`      |     3 | Widget is the active on-screen widget while no scene drag is moving it out. |
| `leaving`      |     4 | Widget is the active widget currently moving off-screen during a drag.      |

The compositor only emits lifecycle for widgets that have a `deck_widget_surface_v1` surface. Widgets still receive the
initial configure batch first; lifecycle is sent after the widget has an attached surface and the compositor has marked
the widget as connected.

## Deriving State

Lifecycle state is derived by `WidgetTracker::lifecycle_states()` from:

- the current scene-cycling list;
- the active scene index;
- the current drag offset, if a scene drag is active;
- the `visible` flag on widgets inside each scene layout.

Automatic scene cycling adds a pre-transition phase before slide motion. During that phase the compositor emits
lifecycle changes first: outgoing widgets become `Leaving`, incoming widgets become `Entering`, and frame callbacks are
kept active so widgets continue animating while the incoming scene warms up. Frame callbacks are held only during slide
motion and resume when the transition reaches a stable state or is cancelled. Neighbour preparation changes for the
post-transition scene are emitted only after the transition commits. The incoming widgets also receive
`transition_incoming`, a one-shot render warm-up event that is not emitted for manual drags.

The derivation is pure scene state. It does not inspect GL state, Wayland buffers, widget runtime state, or whether a
widget has already rendered.

The rules are:

- every visible widget in the scene-cycling list starts as `Dormant`;
- visible widgets in the active scene become `Visible` while idle;
- visible widgets in the active scene become `Leaving` during a non-zero scene drag;
- visible widgets in the immediate previous and next scenes become `Prepared`;
- during a non-zero scene drag, visible widgets in the drag-direction neighbour become `Entering`;
- invisible widgets are not included in the lifecycle map.

In a two-scene cycle, the previous and next neighbour wrap to the same scene. That neighbour is still represented once
and becomes `Prepared` while idle, or `Entering` if it is the drag-direction neighbour.

A touch that has started but has not moved far enough to choose a drag direction has offset `0`. The tracker treats that
as idle, so the active widget remains `Visible` and no neighbour is promoted to `Entering` yet.

## Emission Points

The compositor emits lifecycle in two places.

First, on widget connection:

1. The widget calls `deck_widget_manager_v1.get_widget_surface`.
2. The compositor resolves the instance from the Wayland connection peer PID.
3. The compositor emits the initial configure batch: `configure`, `params`, settings, `configure_done`.
4. The compositor records the connection and later sends the initial lifecycle event.
5. The compositor flushes the affected Wayland client.

If the widget connects before `set_widget_pid` arrives from the coordinator, the compositor buffers the connection by
PID. When the PID registration arrives, it attaches the surface, emits the same initial configure batch, and the normal
connected-widget path sends lifecycle.

Second, after scene state changes:

- `SetActiveScene`;
- `SetSceneCycling`;
- scene drag start, drag motion, drag release, or drag cancel.

Those paths — except drag motion, see below — call `after_scene_change`, which marks compositor output damage and arms a
pending lifecycle emission. The transitions are not sent inline. The compositor loop flushes them — and releases dormant
widgets' buffers — only after it has rendered one frame of the committed scene (`emit_pending_lifecycle`). Deferring the
emission this way keeps a widget host from starting to re-render until the compositor's GPU work for the committed frame
is done, so the host and compositor do not submit to the shared, serialized GPU across the handoff at once. The derived
`WidgetTracker` state is read at emission time, so multiple scene changes before a render coalesce to the latest state.
In headless mode the damage-processing pass stands in for the render and arms the same flush. The initial connect-time
emission above is not deferred.

Drag motion is the exception: `update_drag` calls `after_drag_scene_update`, which emits the transitions inline instead
of arming the deferred flush. Emitting immediately lets a `Visible` widget learn it is `Leaving` before the first drag
frame, so its animation loop stops driving renders that would contend with the drag for the GPU render lock. The inline
path is safe only while the emission carries no transition into `Dormant` — a buffer release must never bypass the
after-render ordering. A drag only moves widgets between render-set states, so no release can ride on it;
`after_drag_scene_update` asserts this no-release invariant. If a deferred emission armed by a prior scene change has
not flushed yet, it may carry `Dormant` transitions, so the drag update leaves it armed instead of emitting inline; the
deferred flush reads the tracker at emission time and coalesces the drag's transitions.

Automatic scene cycling adds a third path. At the start of the pre-transition phase (`BeginPreTransition`), the
compositor emits lifecycle inline through `emit_lifecycle_transitions`, exactly like drag motion and for the same
render-lock reason: outgoing widgets must learn they are `Leaving` and incoming widgets `Entering` before slide motion
starts. This emission carries no transition into `Dormant` — neighbour preparation for the post-transition scene is
withheld until the slide commits — and a `debug_assert!` enforces that no-release invariant. When the slide commits
(`FinishTransition`), that deferred neighbour update flows through `after_scene_change` like any other committed scene
change.

## Initial Lifecycle

The first lifecycle event for a newly connected widget is sent after `configure_done`. It is always an idle state from
the widget's point of view:

- `Dormant`;
- `Prepared`;
- `Visible`.

If the current tracker state would be Entering, the compositor clamps the initial event to Prepared. If it would be
Leaving, it clamps to Visible. Transitional states only make sense as deltas from a prior idle state a brand-new client
cannot have seen.

After sending the initial lifecycle event, the compositor records it in `LifecycleEmitter` so the next regular scene
step does not re-emit the same state.

## Transition Batching

Regular scene changes flow through `LifecycleEmitter`. The emitter keeps the last lifecycle state sent to each widget
instance and computes the delta to the next derived lifecycle map.

It splits transitions into two batches:

1. Release batch: transitions into `Dormant`.
2. Acquire batch: all other transitions.

The compositor sends and flushes the release batch — including `wl_buffer.release` for any buffers it still holds from
the newly dormant widgets — before sending the acquire batch:

```text
send dormant transitions
send wl_buffer.release for the dormant widgets' held buffers
flush affected clients
send acquire/keep-target transitions
flush affected clients
```

This ordering is the important protocol contract. It lets clients that release scarce resources on `Dormant` do so
before other widgets acquire resources for `Prepared`, `Entering`, or `Visible`. Because the WASM host defers buffer
destruction until `wl_buffer.release` arrives, the buffer release must ride in the release batch flush: were it flushed
after the acquire batch, an acquiring slot could allocate while the dormant slot's CMA buffer is still held.

Transitions that do not enter `Dormant` go in the acquire batch. For example, `Visible -> Prepared` and
`Prepared -> Visible` keep the same broad protocol resource class, so they do not need release ordering. The current
WASM host allocates a render target in `Prepared` and pre-renders a single frame, so the immediate neighbour is ready
before a drag; see [`wasm-host/render-loop.md`](wasm-host/render-loop.md).

The emitter sorts entries inside each batch for deterministic behavior, but widget clients should not depend on
inter-widget ordering inside a batch. The externally relevant guarantee is release batch before acquire batch, with a
flush boundary between them.

## Transition Warm-Up

Automatic scene cycling sends `transition_incoming` to visible widgets in the incoming scene during the pre-transition
phase, after the lifecycle acquire batch has made those widgets `Entering`. The event tells hosts to render one fresh
frame before slide motion starts. The compositor waits for that fresh commit before starting slide motion, bounded by a
one-second timeout so a stalled widget cannot stop scene cycling. Frame callbacks remain active throughout warm-up; the
event does not change lifecycle state.

Manual drags do not send `transition_incoming`. Dragging still uses lifecycle `Entering`/`Leaving`, but hosts must not
start animation or warm-up renders merely because a user is dragging through a scene.

## Valid Transitions

The protocol documents these compositor-emitted transitions:

| From       | To                                |
| ---------- | --------------------------------- |
| `Dormant`  | `Prepared`, `Visible`, `Entering` |
| `Prepared` | `Dormant`, `Visible`, `Entering`  |
| `Visible`  | `Dormant`, `Prepared`, `Leaving`  |
| `Entering` | `Visible`, `Prepared`, `Dormant`  |
| `Leaving`  | `Dormant`, `Prepared`, `Visible`  |

Clients should still tolerate repeated states and missing intermediate states. Process restarts, delayed connection,
scene-list replacement, and client disconnects can make a widget's local history shorter than the compositor's scene
history. Treat each lifecycle event as the current truth, not as an animation command that depends on every previous
event having arrived.

## Client Delivery

The generated Wayland client code surfaces lifecycle as ordinary widget events:

- `bmc-widget/src/wayland.rs` pushes `WidgetEvent::Lifecycle(state)`;
- `bmc-widget/src/surface/deck_widget.rs` pushes `DeckWidgetEvent::Lifecycle(state)`, which converts to
  `WidgetEvent::Lifecycle(state)`;
- unknown lifecycle enum values are logged and ignored.

Before `configure_done`, the client is still collecting initial geometry, params, and settings. Lifecycle is not part of
that configure batch; it is delivered as a runtime event after the surface has been configured.

Native widgets that do not implement lifecycle handling continue to work because the default event handler ignores it.
Lifecycle-aware widgets should update their own resource/render policy in response to the latest state.

## Disconnection And Forgetting State

When a widget process exits, the coordinator sends `ClearPid` with the expected PID. The compositor clears that widget's
PID association and forgets its lifecycle-emitter state. This avoids keeping stale lifecycle history across process
restarts and prevents PID reuse from being attributed to the old widget.

`ClearPid` detaches the process, not the instance: the PID and both surfaces go, while the protocol record and its
stored initial config stay. A crash respawn therefore has something to come back to — the coordinator binds the new
process with `BindRespawnedPid` (not `SetWidgetPid`, which registers a fresh spawn), and the reconnect replays the same
configure batch as the first attach, followed by an initial lifecycle event derived from the scene as it stands now. The
bind is dropped unless the instance is still unbound *and* still on the registration the respawn belongs to, since a
scene edit or a widget reload may have re-registered it while the respawn announcement was still queued — and a fresh
registration is itself unbound until its `SetWidgetPid` lands, so unbound alone cannot tell the two apart. See "Crash
supervision" in [`widget-runtime-configuration.md`](widget-runtime-configuration.md) for the generation stamp that
separates them.

When a widget is unregistered, the compositor removes its protocol record and forgets lifecycle state for that instance.
If a widget disappears from the derived lifecycle map during a scene step, `LifecycleEmitter` treats it as transitioning
to `Dormant` and then forgets it.

## Code Map

- `bmc-widget-protocol/protocol/deck-widget-v1.xml` - lifecycle event and enum contract.
- `bmc-openwrt/src/compositor/widget_tracker.rs` - derives lifecycle state from scene cycling and drag state.
- `bmc-openwrt/src/compositor/lifecycle_emitter.rs` - computes release/acquire batches from previous and next state.
- `bmc-openwrt/src/compositor/egl_compositor.rs` - sends lifecycle events, flushes clients, and handles initial
  lifecycle on connection.
- `bmc-openwrt/src/compositor/protocol/state.rs` - attaches widget surfaces, emits the initial configure batch, and
  sends lifecycle events to protocol surfaces.
- `bmc-widget/src/wayland.rs` - native widget client event dispatch.
- `bmc-widget/src/surface/deck_widget.rs` - deck-widget surface client event dispatch used by the WASM host.

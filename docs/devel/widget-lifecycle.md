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

The primary consumer today is the WASM host. It uses lifecycle state to hold render targets only while widgets are
`Entering`, `Visible`, or `Leaving`, and to animate only the `Visible` widget. Native widgets may also consume the event
through `WidgetEvent::Lifecycle`, but widgets that do not care about lifecycle can ignore it.

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
- `SetActiveSceneIndex`;
- scene drag start, drag motion, drag release, or drag cancel.

Those paths call `after_scene_change`, which marks compositor output damage and emits lifecycle transitions derived from
the new `WidgetTracker` state.

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

The compositor sends and flushes the release batch before sending the acquire batch:

```text
send dormant transitions
flush affected clients
send acquire/keep-target transitions
flush affected clients
```

This ordering is the important protocol contract. It lets clients that release scarce resources on `Dormant` do so
before other widgets acquire resources for `Prepared`, `Entering`, or `Visible`.

Transitions that do not enter `Dormant` go in the acquire batch. For example, `Visible -> Prepared` and
`Prepared -> Visible` keep the same broad protocol resource class, so they do not need release ordering. The current
WASM host is more conservative: it does not allocate or render in `Prepared`, but the compositor still emits `Prepared`
so clients can pre-warm immediate neighbours if they support that policy.

The emitter sorts entries inside each batch for deterministic behavior, but widget clients should not depend on
inter-widget ordering inside a batch. The externally relevant guarantee is release batch before acquire batch, with a
flush boundary between them.

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

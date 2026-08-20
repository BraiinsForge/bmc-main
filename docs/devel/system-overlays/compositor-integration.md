# Compositor Integration

This document describes the compositor-side support for system overlays: advertising `wlr-layer-shell`, compositing
layer surfaces above the active scene, tracking and reclaiming their buffers, hit-testing touch, suppressing scene
navigation while an overlay is up, preempting the settings tray for a modal overlay, recognizing the edge-reveal
gesture, and relaying the alarm and upgrade progress. The five protocols are covered in [`protocols.md`](protocols.md);
the client side is in [`framework.md`](framework.md).

All of this lives under `bmc-openwrt/src/compositor/`.

## Advertising layer-shell

Layer-shell is enabled with Smithay's server support: `WlrLayerShellState::new::<Self>(...)` at compositor init, the
`delegate_layer_shell!(CompositorState)` macro, and a `WlrLayerShellHandler` impl on `CompositorState`. A new layer
surface is recorded as a `LayerEntry` in `CompositorState::layer_surfaces`, a `Vec` parallel to the widget bookkeeping;
creating one marks full-output damage.

Surface commits route through `CompositorState::commit`, which checks `commit_layer_surface` first and short-circuits
the widget path when the surface is a layer surface.

## Layers and compositing order

Layer surfaces paint **after** the active scene, alpha-blended, so a transparent panel shows the live scene behind it.
This is deliberate: unlike the standard wlroots convention, all layer ranks paint above the scene here, not just the top
ones.

`layer_rank` (`layer_surface.rs`) maps the ranks `Background → 0`, `Bottom → 1`, `Top → 2`, `Overlay → 3`, and
`paint_order` is a stable sort by rank so later-registered surfaces of the same rank paint on top. The concrete overlays
use this: the offline indicator on `Background`, the startup overlay and the package-upgrade card on `Bottom`, the alarm
and the firmware-upgrade blocker on `Top`, and the swipe panel on `Overlay`. Stacking across ranks is purely by rank,
independent of the order overlays start in.

Within a rank it is not, so `overlay_specs()` in `bmc-wasm-host/src/overlays.rs` is an ordered list and its order is
load-bearing in one place: the firmware blocker registers before the alarm so a firing alarm paints above it. The
startup screen and the package card share `Bottom` too, and the startup screen (registered later) would cover the card —
accepted, because the two cannot realistically be up at once; the reasoning is in [`overlays.md`](overlays.md).

`is_fullscreen_blocker(layer, geo, output)` returns true when a layer surface is above `Background` *and* its geometry
covers the whole output. It is the predicate behind three policies (below): suppressing scene-drag, demoting scene
neighbors, and preempting the settings tray. It keys on *any* full-screen layer surface above background, not a specific
layer, so the full-screen boot screen on `Bottom` blocks scene swipes just as an `Overlay` surface would. `Background`
surfaces are excluded on purpose: they paint above the scene but are passive and must not block scene gestures.

## Layer-surface buffer tracking

The compositor already recorded committed buffers only when it could map a surface to a widget `InstanceId`; layer
surfaces have no `InstanceId`, so they need their own registry. Each `LayerEntry` holds the surface, its layer, the
currently-committed buffer (`Option`), the buffer's `ObjectId`, and the last geometry.

At commit time `commit_layer_surface` processes the buffer assignment:

- **New buffer** — record the new geometry, enqueue the *old* buffer for `wl_buffer.release` and its `ObjectId` for
  texture invalidation, and mark the new buffer's `ObjectId` dirty for import.
- **NULL buffer (unmap)** — release the old buffer and invalidate its texture; **`last_geometry` is left in place** (it
  still backs touch hit-testing). Repainting the vacated region is covered by the full-output damage every layer commit
  forces, below.

The retained `buffer_id` is what makes NULL-buffer eviction possible: an unmap commit carries no buffer object, so
without the stored `ObjectId` there would be nothing to match against the renderer's `texture_cache`. This is the
concrete backing for the framework's "buffers while hidden" guarantee — see [`framework.md`](framework.md). Previously
the compositor invalidated a cached import only on buffer-*destroy*; the unmap path extends that so the DMA-BUF is
reclaimed on both sides.

Hiding an overlay must repaint where it was. Every layer commit — map, unmap, or geometry change — forces full-output
damage, so the scene shows again with no stale overlay pixels. The partial bottom-right indicator is the sharp case:
hiding it must repaint its corner, not merely stop drawing it.

## Touch hit-testing

`touch_focus_at(x, y)` checks layer surfaces before scene widgets. It walks the mapped layer entries topmost-first
(reverse of `paint_order`) and, for each, requires the point to be inside `last_geometry`, the surface to be alive, and
the point to fall inside the surface's input region. The layer-shell input-region semantics are honored directly: an
unset region accepts all input, an explicitly empty region accepts none (touches fall through to whatever is behind it —
this is what makes the offline indicator passive). Only if no layer surface claims the point does it fall through to the
active scene's widgets.

## Suppressing scene navigation

While a full-screen overlay or a revealed edge panel is up, horizontal scene-swipe must not run, and the scene's swipe
neighbors should release their buffers rather than stay pre-rendered. Both are driven by one predicate: scene navigation
is blocked when a full-screen blocker is active **or** any screen edge is currently revealed. On that condition the
compositor demotes the would-be `Prepared` neighbors to `Dormant`, freeing their buffers since swiping is disabled
anyway.

## Modal-overlay preemption

The settings tray sits on the top (`Overlay`) layer, painted above everything so the user can pull it down over any
content — but it must *yield* when a modal full-screen overlay takes the screen (a firing alarm; the startup screen).
The compositor owns this because it is the single source of truth for the layer stack.

`modal_overlay_active()` is the predicate: a mapped `is_fullscreen_blocker` on a layer *below* `Overlay` (so the tray
never counts itself). This is purely geometric — any full-screen overlay below the tray qualifies, so a new modal
overlay needs no wiring here and the tray needs no per-feature knowledge. Today the alarm, the startup screen, and the
firmware-upgrade blocker match; the startup screen only ever maps at boot, before the tray can be pulled down. The
firmware blocker is the payoff for keeping this geometric: it preempts the tray without a line of upgrade-specific code
here or in the tray. The package card does not qualify and must not — it is a corner surface that deliberately leaves
the scene usable.

Like the neighbor-suppression check, a modal map/unmap happens during dispatch, not on a scene command, so the main loop
compares `modal_overlay_active()` against its last value each iteration and, on the edge, emits `deck_settings_v1`'s
`preempted(active)` to the tray. The tray retracts on `preempted(1)` (see [`overlays.md`](overlays.md)). The signal is
edge-triggered and not cached for bind replay: the tray binds at startup before any modal maps, and a preemption while
the tray is hidden is harmless (a screen-edge overlay stays unmapped unless revealed). Deliberately *not* used for this:
tying it to the alarm specifically — that would make every future modal overlay re-plumb the tray.

## Edge-reveal gesture

The swipe-from-edge reveal needs compositor-owned gesture detection. The gesture state (`touch_gesture.rs`) gains a
top/bottom edge candidate distinct from the horizontal scene drag. Its tuning constants:

| Constant                 | Value | Meaning                                                                    |
| ------------------------ | ----- | -------------------------------------------------------------------------- |
| `EDGE_HOT_ZONE_FRACTION` | 0.20  | The touch-down must start within the top (or bottom) 20% of the screen.    |
| `EDGE_ACTIVATION_DY`     | 40.0  | Downward (top) / upward (bottom) travel, in px, that activates the reveal. |
| `EDGE_MAX_X_DEVIATION`   | 150.0 | Horizontal drift budget; a diagonal swipe past this is not an edge reveal. |

Unlike the scene drag, which can begin anywhere on the screen, the edge gesture is deliberately edge-anchored so it does
not conflict with normal scene interaction. The recognizer checks the edge candidate **before** horizontal drag, so a
sufficiently vertical top-edge swipe wins even when its horizontal drift exceeds the scene-drag dead zone
(`DRAG_DEAD_ZONE = 15.0`); a mostly horizontal swipe in the top band still navigates scenes.

### Forward-then-cancel arbitration

The compositor reuses the exact touch arbitration the scene drag already uses. A touch-down inside the hot zone is still
forwarded immediately to the focused surface while the recognizer evaluates the candidate. If a reveal activates, the
compositor triggers the armed edge, latches the reveal, and sends `wl_touch.cancel` to the surface that had the
sequence, then owns the gesture (it simply reveals the panel — there is no finger-tracking). If the trigger fails or the
gesture does not activate, the edge candidate is rejected and the surface keeps the sequence as a normal touch. A
touch-down outside the hot zone never participates in the reveal. This keeps a single defined owner of the touch at
every moment and adds no separate input path.

## `deck_screen_edge_v1` dispatch

The compositor creates the `deck_screen_edge_v1` global and tracks sessions. `get_auto_hide_screen_edge` validates that
the surface already has the layer-surface role (else `invalid_role`) and rejects a second registration for the same
surface (else `already_constructed`). Each session carries an `EdgeFlags { border, armed, revealed }`:

- `activate()` arms the edge and requests hide — sets `armed`, clears `revealed`, emits `hidden`.
- `deactivate()` shows the surface explicitly — clears `armed`, sets `revealed`, emits `revealed`, and damages the
  output.
- An edge gesture calls `try_trigger(border)`, which fires only when the border matches and the edge is armed; it then
  spends the arming (`armed = false`, `revealed = true`) and emits `revealed`. A spent edge must be re-armed with
  `activate()`.

## `deck_settings_v1` dispatch

The compositor creates the `deck_settings_v1` global and relays between the settings-tray overlay and bmc. Incoming
requests are queued and drained each loop pass into `SettingsCommand`s sent to bmc over the existing action channel:
`set_brightness(value)` is clamped to 0–100 first; `reconfigure_wifi` is forwarded as-is. Outgoing events
`brightness(value)` and `wifi_ap(ssid)` are emitted whenever bmc broadcasts a change, and the last value of each is
cached so a late-binding overlay receives the current state immediately on bind. Brightness is intentionally cached only
once a real value exists, so a cold cache does not snap the overlay's slider to 0 on bind; the WiFi-AP value is replayed
unconditionally (empty string when setup mode is inactive). The protocol shape and the reason bmc — not the overlay —
owns brightness are in [`protocols.md`](protocols.md).

The `preempted` event does not come from bmc: `set_preempted(active)` is called from the loop's modal-preemption edge
check (above) and fans out to bound v3 resources. It is not cached — see the modal-preemption section for why edge-only
emission is correct.

## `deck_alarm_v1` dispatch

The compositor creates the `deck_alarm_v1` global and relays between bmc's alarm domain and the alarm overlay, mirroring
the settings dispatch. `AlarmState` tracks the bound overlay resources, a `pending_actions` buffer, and a `ringing`
flag:

- Outgoing: `AlarmState::ring(time, label, snooze_allowed)` fans out `alarm_ringing` to every live resource and sets
  `ringing`; `stop()` fans out `alarm_stopped` and clears it. Both prune dead resources first, so a client that vanished
  without `destroy` is reaped on the next emit.
- Incoming: `snooze_alarm` / `dismiss_alarm` requests are buffered as `AlarmAction`s and drained each loop pass into
  lossless `AlarmCommand`s on a dedicated mpsc channel to bmc (not the lossy broadcast). `destroy` removes the session
  by resource identity.

bmc drives `ring` / `stop` through the `Compositor::broadcast_alarm_ring` / `broadcast_alarm_stop` trait methods (a
`bmc` startup task bridges `AlarmBus` events and the overlay); the drained commands flow back as `AlarmCommand::Dismiss`
/ `Snooze`. See [`protocols.md`](protocols.md) for the responsibility split.

### No-overlay / crash fallback

An alarm must never ring with no way to silence it, even if no overlay is bound at fire time or the overlay client dies
mid-ring. When `ring` fires the loop arms a watchdog (`arm_alarm_fallback`): it seeds a "no live overlay since" instant
if none is bound, then polls every `ALARM_FALLBACK_POLL` (1 s). `AlarmState::has_live_overlay()` reports whether any
bound resource is still alive; with none for `ALARM_FALLBACK_GRACE` (2 s) the compositor auto-dismisses (queues a
`Dismiss` as if the overlay had requested it). Independently, while an alarm is ringing with no live overlay, *any*
touch dismisses it immediately (and is consumed, not routed into gestures). `stop` / dismissal cancels the watchdog.
This is why only the alarm overlay binds `deck_alarm_v1`: `has_live_overlay()` counts bound resources, so a passive
listener binding the protocol would mask a real crash — the settings tray instead learns about a firing alarm through
the generic `deck_settings_v1` preemption above, not by binding `deck_alarm_v1`.

## `deck_device_info_v1` dispatch

The compositor creates the `deck_device_info_v1` global and fans bmc's device lifecycle out to every bound client
(`device_info.rs`). It carries no incoming requests: bmc owns the lifecycle and its recovery policies, and the overlay
only renders.

bmc pushes state through three `Compositor` trait methods — `broadcast_device_state`, `broadcast_setup_progress`, and
`broadcast_access_point` — which arrive as `CompositorCommand`s and land in `DeviceInfoState`. The last value of each
event is cached and replayed on bind (the `device_state` only once bmc has reported one, so an early-bound overlay waits
instead of guessing). Two things the cache does beyond storing the latest value, both so a *restarted* overlay is not
handed a past moment as if it were current: the `setup_progress` replay downgrades announcement steps to `idle`, and
`device_state` carries a `boot_flow_delivered` flag that latches once an operational state has reached a client. See
[`protocols.md`](protocols.md) for the reasoning. The events are fed by the device-info listener in
`bmc/src/startup.rs`, which mirrors the `BmcState`, forwards `InitialSetup` transitions the moment they happen, resolves
the setup-AP SSID/URL when the AP watch flips, and carries the stable-26.02 recovery policies — broadcasting
`unexpected_error` before acting, flagged with whether it is about to restart the device. Resolving the AP runs in its
own task: the SSID and address waits together run to half a minute, and the listener has to keep forwarding setup
transitions while they do.

## `deck_upgrade_v1` dispatch

The compositor creates the `deck_upgrade_v1` global and fans bmc's upgrade display projection out to every bound client.
It carries no incoming requests: bmc owns upgrade decisions, and the overlays only render.

bmc pushes state through the `Compositor::set_upgrade_state(UpgradeDisplaySnapshot)` trait method, which arrives as a
`CompositorCommand::SetUpgradeState` and lands in `UpgradeState::set`. Each snapshot wholly replaces the previous one —
there is no incremental update path, which is what lets a client discard a malformed sequence and keep its last coherent
view.

`UpgradeState` splits into a pure `UpgradeCache` (snapshot plus deadline, unit-testable without Wayland resources) and
the resource list. Two behaviors live in the cache:

- **Bind replay.** The current snapshot is cached and replayed to each newly bound resource. This is the opposite of
  `deck_settings_v1`'s `preempted`, which is deliberately edge-only: the tray is guaranteed to be bound before any modal
  maps, whereas upgrade overlays and the startup screen bind at their own times and must be able to learn about a run
  already in progress.
- **Deadline ownership.** A terminal snapshot gets a deadline of `now + TERMINAL_LIFETIME` on first arrival, and a later
  snapshot of the *same* `generation` reuses it rather than restarting it, so a coalesced re-send cannot extend the
  screen. Each replay recomputes `remaining_ms` against that deadline; once it has passed, `events()` yields nothing and
  the terminal state is simply not replayed.

`generation` is what distinguishes a re-send of the current run from a genuinely new one: a new generation replaces an
expired terminal snapshot, and a new *running* generation replaces a terminal snapshot without emitting terminal events
for it.

Snapshots are serialized into a `WireEvent` list bracketed by `started` / `snapshot_done` (see
[`protocols.md`](protocols.md)) and emitted to each live resource, pruning dead ones first, as the alarm dispatch does.

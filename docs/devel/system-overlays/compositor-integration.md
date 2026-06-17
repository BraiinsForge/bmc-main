# Compositor Integration

This document describes the compositor-side support for system overlays: advertising `wlr-layer-shell`, compositing
layer surfaces above the active scene, tracking and reclaiming their buffers, hit-testing touch, suppressing scene
navigation while an overlay is up, and recognizing the edge-reveal gesture. The two vendored protocols are covered in
[`protocols.md`](protocols.md); the client side is in [`framework.md`](framework.md).

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
use this: the offline indicator on `Bottom`, the startup overlay on `Top` (opaque, occludes lower layers), and the swipe
panel on `Overlay`. Stacking is purely by rank, independent of the order overlays start in.

`is_fullscreen_blocker(layer, geo, output)` returns true when a layer surface is above `Background` *and* its geometry
covers the whole output. It is the predicate behind two policies (below): suppressing scene-drag and demoting scene
neighbors. It keys on *any* full-screen layer surface above background, not a specific layer, so the full-screen boot
screen on `Top` blocks scene swipes just as an `Overlay` surface would.

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

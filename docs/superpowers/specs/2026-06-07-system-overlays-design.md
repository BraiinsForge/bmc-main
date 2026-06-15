# System overlays framework — design

Status: approved (brainstorming), pending implementation plan Date: 2026-06-07 Ticket: BDK-416 Branch:
fbo/BDK-XXX/system-stuff

## Context

The product is migrating from a single Slint monolith to a granular architecture: a custom Smithay-based Wayland
compositor (`bmc-openwrt/src/compositor/`) hosting widgets, with WASM widgets running inside a shared `bmc-wasm-host`
daemon (one per SDK major version, started by the first widget via `bmc-wasm-thin`). Widgets speak a custom
`deck_widget_v1` protocol (`bmc-widget-protocol/`) and are composited as a single active scene at a time using a
painter's-algorithm pass in `scene_renderer`.

We now need "system overlays" that are not widgets: initial WiFi setup over AP, WiFi reconfigure, alarms, a
swipe-from-top quick-settings panel, an "offline" status indicator, and a mining status indicator. Some of these can be
driven entirely by client-side logic; others (the swipe-from-top reveal) genuinely require compositor support to detect
the gesture and route touch.

## Goals

- A generic, modular framework for system overlays, reusable across all of the above.
- Overlays are privileged native components (no WASM sandbox) that can do arbitrary background work and ask to be
  rendered.
- For memory reasons the overlays compile into `bmc-wasm-host` for now and share the host's renderer, while remaining
  capable of running as standalone applications.
- The most important shared resource is the renderer (the GL context and font cache).

## Non-goals (deferred)

- Compositor↔overlay IPC for compositor-owned state such as screen brightness. The compositor owns brightness; an IPC
  will be needed eventually, but not in this work. Step-2 overlays read what they need directly from the OS.
- Concrete UIs for alarms and WiFi-AP setup. These are later overlays that reuse this framework.
- Finger-tracking during the swipe reveal. The gesture only triggers "appear"; a short reveal animation is optional.

## Ecosystem precedent

The Wayland world splits into two camps. Monolithic shells (GNOME/mutter, gamescope) keep shell UI inside the compositor
and refuse `wlr-layer-shell`. Modular shells (the wlroots ecosystem — sway, wayfire, labwc — plus phosh and KDE Plasma)
implement panels, notification centers, OSDs, launchers, and lock screens as separate `wlr-layer-shell` clients. The
modular approach is the mainstream, well-supported path and matches our modularity goal.

The swipe-from-edge reveal is also solved prior art: KDE's `kde-screen-edge-v1` lets a client register a layer-shell
surface as an auto-hide edge panel; the compositor detects the edge gesture and reveals the surface. KWin does gesture
detection in the compositor (a `GestureRecognizer` per screen edge), with the client uninvolved in detection. This is
the division we adopt.

References:

- kde-screen-edge-v1: https://wayland.app/protocols/kde-screen-edge-v1
- KWin touch screen-edge gestures:
  https://blog.martin-graesslin.com/blog/2017/04/how-input-works-touch-screen-edge-swipe-gestures/
- wlr-layer-shell: https://wayland.app/protocols/wlr-layer-shell-unstable-v1

## Architecture

### Core model

System overlays are `wlr-layer-shell` clients, distinct from `deck_widget_v1` widgets. Generic plumbing lives in a
framework crate; each concrete overlay is its own small crate implementing a trait.

Each overlay crate always opens its own Wayland connection — it is genuinely a separate client from the compositor's
view, in both run modes. Only two things differ by mode:

|                    | Standalone            | Hosted (in `bmc-wasm-host`)                                         |
| ------------------ | --------------------- | ------------------------------------------------------------------- |
| Wayland connection | own                   | own (separate `wl_display` in the host process)                     |
| Renderer           | own `FemtoVgRenderer` | borrows the host's shared `FemtoVgRenderer` for one render callback |
| Event loop         | own poll loop         | driven by the host main loop                                        |

The expensive, memory-bearing GL context and font cache are shared in the hosted case; everything else is identical
code.

The host owns the shared renderer and lends it to exactly one overlay at a time, only for the duration of that overlay's
render callback. The host iterates its overlays (and widget slots) in a single loop, rendering them one after another,
so the renderer is never used by two components concurrently — the same single-user guarantee the host already relies on
for WASM widgets; overlays do not change it. The unsafe `NonNull<dyn Renderer>` reborrow is a host-internal
implementation detail (see the aliasing invariant in `bmc-wasm-host/src/host.rs`) and is deliberately not part of the
overlay-facing API; the host hands a plain `&mut dyn Renderer` into the callback.

### Why layer-shell and not `deck_widget_v1`

`deck_widget_v1` is the widget protocol and carries widget semantics (scene placement, params, lifecycle tied to scene
cycling). System overlays are not widgets, are not placed in scenes, and need z-ordering above scenes plus edge
anchoring — exactly what layer-shell provides. Smithay ships `wlr-layer-shell` (`src/wayland/shell/wlr_layer/`,
`delegate_layer_shell!`, `WlrLayerShellState`), so we get layers, anchors, exclusive zones, and input regions for free.

### Framework crate (`bmc-system-overlay`, name TBD)

Provides:

- Layer-shell client plumbing: connect, create a `zwlr_layer_surface_v1`, set layer/anchor/exclusive-zone/size, and
  manage per-surface DMA-BUF export buffers (mirroring the widget slot's double-buffer in `bmc-wasm-host`).
- A `SystemOverlay` trait the concrete crates implement: `init`, a `tick`/poll for background work that reports whether
  it wants to render, `render(&mut dyn Renderer)`, and input handling. The `&mut dyn Renderer` is valid only for the
  duration of the `render` call: the trait must not store it or call back into the host renderer outside that callback
  (no retained or reentrant use). `NonNull` never appears in this API.
- Rendering uses the declarative tree pipeline (the same `col`/`row`/`text`/`button`/`canvas` model as WASM widgets,
  laid out host-side with Taffy and drawn with femtovg) by default, with the `canvas` immediate-mode element as the
  escape hatch for gesture-driven and custom visuals. Native overlays build the tree structs directly and call
  `bmc-render` layout/draw without the WASM serialization step.
- A "request render next pass" signal, equivalent to the widget `request_frame` mechanism.
- Two entrypoints: `standalone` (owns connection, renderer, and loop) and `hosted` (the host lends `&mut dyn Renderer`
  and ticks the overlay).

### Protocol: `deck_screen_edge_v1` (vendored and forked from `kde-screen-edge-v1`)

`kde-screen-edge-v1` is not implemented in Smithay (the `kde` shell module covers only server-side decorations) and
there is no near-equivalent to wire up faster — the reveal logic is custom compositor code regardless. We vendor the
protocol, matching the existing `bmc-widget-protocol` convention (`protocol/*.xml` plus
`wayland_scanner::generate_server_code!`), so we own it and can modify it. The `wayland-protocols-plasma` crate stays a
reference for the XML, not a runtime dependency.

Because we change its contract (added `revealed`/`hidden` events and a different buffer-retention semantic — see below),
the vendored copy is renamed to `deck_screen_edge_v1` rather than keeping the `kde_screen_edge_v1` interface name.
Keeping the upstream name would mislead the next reader into assuming upstream semantics; the `deck_` prefix follows the
`deck_widget_v1` precedent of not impersonating someone else's protocol. Where this document says `kde-screen-edge-v1`,
it means the upstream protocol we forked from.

How the protocol works, and the split of responsibility:

- Layer-shell owns placement. `get_auto_hide_screen_edge(id, border, surface)` raises `invalid_role` unless `surface`
  already has the `layer_surface` role. The layer surface's anchor/size/exclusive-zone fix where it lives; the
  `wl_surface` argument is only an association.
- screen-edge owns visibility and the trigger. `activate()` hides the surface and arms the edge; the compositor reveals
  it on the edge gesture (the edge is then spent and must be re-armed); `deactivate()` shows it explicitly.

Our extensions, the reason for vendoring:

- The upstream protocol has no events — the compositor un-hides silently. We add a `revealed`/`hidden` event so the
  in-host overlay knows when to start ticking/animating and so the framework can drive the neighbor→Dormant demotion.
- We tie the "hidden" state to dropping the overlay's buffers (allocate-on-reveal, free-on-hide); see the memory note
  below.

### Memory: buffers while hidden

A hidden overlay must not hold a fullscreen buffer. A `wl_surface` can have no buffer at all: committing a NULL buffer
unmaps the surface, so there is no content and no allocation. We use this for both the swipe panel (while not revealed)
and the startup IP surface (after it dismisses).

This diverges from the vanilla `kde-screen-edge-v1` model, which assumes the surface keeps its buffer while hidden so
the compositor can reveal it instantly. We trade that instant reveal for zero allocation while hidden: on the edge
trigger the compositor emits the `revealed` event, the overlay allocates, renders, and attaches a buffer, and the
compositor maps it. The cost is a small reveal latency (one allocation plus the first frame, possibly a configure
round-trip on re-map), which is acceptable here because the reveal animation is optional and memory is the priority.

Freeing on hide takes both sides. On `hidden` the overlay attaches a NULL buffer and frees its DMA-BUFs, and the
compositor must, on that unmap, drop its imported texture for the surface (the `texture_cache` keyed by buffer
`ObjectId` in `scene_renderer`) and send `wl_buffer.release`. Otherwise the compositor's cached import keeps the buffer
referenced and the memory is never reclaimed. Today this invalidation happens only on buffer-destroy; we extend it to
the NULL-buffer unmap.

The client must also treat NULL-buffer unmap configure events carefully. Layer-shell state is reset on unmap, so before
the next real buffer attach the client reapplies its layer/anchor/size/margins/input-region state. Some compositor flows
can report a placeholder configure while the surface is unmapped (observed as `1x200` for a full-width top strip); that
event should be drained for protocol correctness but must not resize the reusable render target. The next mapped frame
keeps the last usable configured size and commits the restored layer-shell pending state with the new buffer.

### GPU render serialization and GL fences

The GPU is an etnaviv Vivante part running MMUv2 with per-context address spaces. Cross-context in-flight job
interleaving causes scene-freeze MMU faults, worked around by a cross-process render lock at `/run/bmc-gpu-render.lock`
and by waiting on the host GL fence before handing a buffer to the compositor (the recent BDK-509 work on this branch:
serialized GPU render submits, host-GL-fence-before-handoff). This subsystem is sensitive and was recently destabilized,
so overlays must fit the existing discipline rather than open a new hole. This is a first-class constraint, not an
afterthought.

- Hosted mode (the current target): overlays render through the host's shared renderer inside the host's existing render
  loop and GL context. They add no new GPU context and ride the host's existing serialization and fence handoff — an
  overlay's buffer must be published to the compositor under the same GL-fence-before-handoff rule as a widget's. This
  must be verified, not assumed: the per-surface export/fence path the host uses for widget slots has to cover overlay
  surfaces too.
- Standalone mode: a standalone overlay is a separate process with its own GL context — exactly the cross-context GPU
  client the render lock exists for. It must take `/run/bmc-gpu-render.lock` around its submits, like the compositor and
  host do. The framework's standalone entrypoint owns acquiring it.
- Compositor alpha-blend pass: compositing an overlay surface over the live scene samples the overlay's buffer, so it
  must respect the same GL-fence handoff the compositor already applies to widget buffers before sampling. The new
  alpha-blend path must not bypass the fence wait.

Because the immediate work targets hosted overlays, the main new surface area is verifying overlay buffers ride the host
fence path and that overlay-compositing waits on the fence. The standalone lock requirement is recorded here so it is
not rediscovered the hard way later.

### Compositor changes

- Enable Smithay's `delegate_layer_shell!` / `WlrLayerShellState`.
- Composite layer-shell surfaces above the active scene in `scene_renderer`, alpha-blended so a transparent panel shows
  the live scene behind it. All layer ranks paint above the scene (the standard wlr "Bottom below normal content"
  convention does not apply here).
- Layer assignment: reserve `Layer::Overlay` for the swipe panel, place the fullscreen startup overlay on `Layer::Top`
  (opaque, so it occludes lower layers), and the passive bottom-right indicators on `Layer::Bottom`. Scene-drag
  suppression keys on *any* fullscreen layer surface above `Background` (the `is_fullscreen_blocker` predicate), not a
  specific layer, so the fullscreen boot screen on `Top` still blocks scene swipes behind it.
- Release the overlay's buffer when it is hidden: on a NULL-buffer unmap, drop the cached imported texture for that
  surface and send `wl_buffer.release`, so the DMA-BUF is reclaimed on both sides (extending the current
  invalidate-on-buffer-destroy path).
- Extend `touch_focus_at` so layer surfaces hit-test before scene widgets when present.
- Vendor the protocol as `deck_screen_edge_v1` and hand-write the server `Dispatch`/`GlobalDispatch`; emit the added
  `revealed`/`hidden` events.
- Add a top-edge vertical gesture to the existing gesture state (`touch_gesture.rs`), distinct from the horizontal
  scene-swipe, that triggers the armed edge. This gesture must be initiated at the top edge: the touch-down has to start
  within the top 20% of the screen. This is unlike the scene drag (left/right), which can begin anywhere on the screen;
  the edge gesture is deliberately edge-anchored so it does not conflict with normal scene interaction. A downward
  reveal may drift horizontally by up to 150 logical pixels. The recognizer checks the edge reveal before horizontal
  scene drag, so a sufficiently downward top-edge swipe wins even when its horizontal drift is larger than the
  scene-drag dead zone; a mostly horizontal swipe in the top band still navigates scenes.
- When an overlay is revealed, demote scene-swipe neighbors to `Dormant` rather than keeping them `Prepared`, releasing
  their buffers since scene-swiping is disabled while the overlay is up. The exact `Prepared`-vs-`Dormant` semantics are
  to be confirmed against the current lifecycle code during planning; the intent is to release neighbors more
  aggressively.

### Compositor: layer-surface buffer tracking

Today the compositor records a committed buffer only when it can map the surface to a widget `InstanceId`
(`widget_buffers` in `state.rs`); a surface it cannot map marks full damage and drops the buffer path, and the renderer
imports only from `widget_buffers` (`scene_renderer.rs`). Layer surfaces have no `InstanceId`, so none of this covers
them yet. The memory and compositing guarantees above therefore require, compositor-side:

- A layer-surface buffer registry parallel to `widget_buffers`, keyed by the layer `wl_surface`, recording the
  currently-committed buffer (or none when unmapped).
- A surface→buffer-`ObjectId` mapping maintained at commit time, so that on a NULL-buffer unmap — where the commit
  carries no buffer object — the unmap path can still find and evict the matching `texture_cache` entry. Eviction must
  not depend on the buffer object still being present.
- Dirty/damage tracking for layer surfaces feeding the existing damage model. Distinguish the transition from the steady
  state: the unmap (hide) must deliberately damage the region the overlay vacated — either the overlay's last bounds or,
  simplest and matching the renderer's existing fallback when no damage rect is available (`scene_renderer.rs`), a
  full-output damage for that frame — so the scene repaints where the overlay was. Once hidden, the overlay contributes
  no ongoing damage. The same applies to a partial (non-fullscreen) overlay such as the bottom-right indicator: hiding
  it must repaint its corner, not just stop drawing it.
- Buffer release and texture invalidation: `wl_buffer.release` on replace and on unmap, and `texture_cache` eviction on
  unmap (per the memory section).

This is the concrete backing for the memory guarantee; without it the NULL-buffer-hide path has nothing to evict and the
import path has nothing to render.

### `bmc-wasm-host` changes

- The main loop gains a step that ticks registered in-host system overlays alongside widget slots, renders the ones that
  want it via the shared renderer, and submits to their own layer surfaces.
- These overlays are compiled in as privileged native components, not loaded as WASM.

### Input: regions and arbitration

Each overlay sets its layer-surface input region explicitly. The layer-shell default (the whole surface accepts input)
is the wrong default for a passive indicator, so this is specified per overlay:

- Startup IP overlay (fullscreen): full input region. It blocks scene touch while shown and dismisses on tap or after
  the success/failure display timeout. While unmapped it accepts nothing.
- Offline indicator (bottom-right): empty input region. It is purely passive and must not eat touches in its corner;
  touches there fall through to whatever is behind it.
- Mining indicator (later, bottom-right): same as the offline indicator — empty input region.
- Swipe panel: input region covering the panel while revealed; empty/unmapped while hidden. The reveal gesture itself is
  owned by the compositor (below), not by the panel's input region.

Touch arbitration for the top-edge reveal. Today touch-down is forwarded immediately to the focused surface
(`egl_compositor.rs`), and only after horizontal-drag activation does the compositor cancel the widget's sequence
(`wl_touch.cancel`) and take over for the scene swipe; the gesture state is horizontal-only (`touch_gesture.rs`). We
reuse that exact forward-then-cancel arbitration for the edge reveal, extending `GestureState` with a top-edge candidate
whose touch-down must start within the top 20% of the screen and whose activation is a downward motion. A touch-down
inside the top hot zone is still forwarded to the focused surface while `GestureState` evaluates the candidate; if a
downward reveal activates, the compositor cancels that sequence and owns the gesture (then simply reveals the panel - no
finger-tracking); if it does not, the surface keeps the sequence as a normal touch. A touch-down outside the hot zone
never participates in the reveal. The reveal path allows a horizontal drift budget (`EDGE_MAX_X_DEVIATION = 150`) and
relies on edge-before-drag precedence plus the activation latch, not on `dx <= DRAG_DEAD_ZONE`, for arbitration. This
keeps a single, defined owner at every moment and adds no separate touch path.

### Data and system access

Step-2 overlays read what they need directly from the OS: interface addresses and connectivity via system APIs
(`getifaddrs`, preferring `wlan*` station interfaces), and the saved station SSID via OpenWrt's `uci` CLI. "Online" is
defined as routable-IPv4 presence — not carrier state or uplink reachability. The compositor↔overlay IPC for
compositor-owned state (brightness) is deferred until the first overlay that needs it.

## Implementation steps and crates

1. Framework: compositor `wlr-layer-shell` support, alpha-blended overlay compositing, the layer-surface buffer
   tracking, `bmc-wasm-host` integration, and the `bmc-system-overlay` framework crate with both entrypoints. Step 1
   ships a minimal throwaway validation overlay (a static surface) so the framework is verified end-to-end independently
   of network state; it is removed once Step 2's IP overlay lands.
2. Overlays that need no compositor signal (pure layer-shell, host-logic-driven, direct OS reads):
   - `bmc-overlay-offline` (name TBD): a bottom-right "offline" indicator shown when neither WiFi nor ethernet is
     connected (operationally: no routable IPv4 present).
   - `bmc-overlay-ip` (name TBD): a fullscreen startup overlay that maps immediately at operational startup and shows
     WiFi/IP connection progress — the configured station SSID while waiting, then the device IP on success, or a
     failure message if no routable address arrives within the wait window. It mirrors the legacy boot-status screen
     (`display_tasks.rs`), observes saved WiFi config and IP state only (no initial-setup connect flow), and dismisses
     on tap or after the success/failure display timeout, then unmaps for the session.
3. `deck_screen_edge_v1`: vendored-and-renamed protocol (forked from `kde-screen-edge-v1`) with the `revealed`/`hidden`
   extension, compositor `Dispatch`, the top-edge gesture, and the neighbor→Dormant demotion.
4. Drag overlay: `bmc-overlay-quick-settings` (name TBD), the swipe-from-top transparent panel built on the Step-3
   screen edge. No finger-tracking; short reveal animation moving the buffer.

The offline status, startup IP, and drag overlay are each separate small crates.

### Repository layout

All system-overlay crates are grouped under a single top-level folder (e.g. `system-overlays/`, name to confirm),
mirroring how `widgets/` and `widgets-wasm/` group their crates. This holds both the framework crate
(`bmc-system-overlay`) and the per-overlay crates (offline status, startup IP, drag overlay, and later additions).

The vendored `deck_screen_edge_v1` protocol crate does not live under the overlay folder. It is a protocol crate shared
between the compositor and the overlay framework, so it sits at the workspace root alongside `bmc-widget-protocol` —
protocol crates stay grouped together, and the compositor does not depend into the overlay folder. This is decided now
(not deferred) because it shapes the dependency graph for Steps 3 and 4.

## Verification

Each step lands with tests; the cross-cutting ones called out here because they are easy to skip and costly to regress:

- Gesture state (pure logic, no GPU): top-edge path - touch-down inside vs outside the top 20% hot zone, downward swipe
  activates, diagonal downward swipe with horizontal drift still activates, horizontal/tap does not, and the
  forward-then-cancel arbitration transfers ownership correctly.
- NULL-buffer-unmap → texture-cache invalidation: an unmap evicts the `texture_cache` entry and releases the buffer.
  This is a memory-reclaim correctness change, and the current code invalidates only on buffer-destroy, so the new edge
  regresses silently if untested.
- Layer-surface buffer registry: commit/replace/unmap transitions (buffer recorded, replaced buffer released, unmap
  clears the registry and evicts the texture).
- Hide repaints the vacated region: after an overlay hides, the region it occupied shows the scene again with no stale
  overlay pixels (the partial bottom-right indicator is the sharp case — verify its corner repaints, not just that
  drawing stops).
- Input regions: the offline indicator's empty input region lets touches fall through; the fullscreen IP overlay blocks
  them.
- Reveal latency (once Step 4 exists): measure allocate-on-reveal latency (allocation + first frame + any re-map
  configure round-trip) on the device rather than assuming it is acceptable; revisit keep-buffer-while-hidden for the
  swipe panel only if it proves too slow.
- GPU/fence (on-device): hosted overlay buffers ride the host GL-fence handoff and alpha-compositing waits on the fence
  — no MMU-fault regression under the BDK-509 conditions.

## Open questions

- Final crate names.
- Exact `Prepared`-vs-`Dormant` lifecycle semantics, to be confirmed against the current code during planning.

## Resolved

- The fullscreen IP overlay dismisses on tap **or** after the success/failure display timeout (Step 2).

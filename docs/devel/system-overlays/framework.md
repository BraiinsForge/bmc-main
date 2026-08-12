# System Overlay Framework

The `bmc-system-overlay` crate is the shared plumbing every overlay builds on. It owns the layer-shell client, the
double-buffered DMA-BUF render target, the GPU-fence handoff, the declarative tree UI, the connectivity prober, and both
run-mode entrypoints. A concrete overlay is a small crate that implements one trait and ships a `main.rs` calling
`run_standalone`.

For where overlays sit in the system and how the two run modes differ, see [`README.md`](README.md).

## The `SystemOverlay` trait

An overlay implements `SystemOverlay` (`system-overlays/bmc-system-overlay/src/overlay.rs`). Only three methods are
required; the rest have defaults so a passive overlay stays small.

| Method                                                                                                     | Required | Purpose                                                                                                                                              |
| ---------------------------------------------------------------------------------------------------------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `layer_config`                                                                                             | yes      | Static placement and input policy (layer, anchor, size, margins, namespace, input region).                                                           |
| `tick(now) -> TickOutcome`                                                                                 | yes      | Per-pass background work; reports whether it wants to be on-screen, wants a redraw, and when to wake next. Must not block.                           |
| `render(renderer, size)`                                                                                   | yes      | Draw the frame. The `&mut dyn Renderer` is valid only for this call.                                                                                 |
| `init`                                                                                                     | no       | Called once before the first render.                                                                                                                 |
| `prewarm(renderer)`                                                                                        | no       | Pay one-time renderer setup (SVG icon decode, glyph atlas) at host startup, not on first reveal.                                                     |
| `on_touch(event)`                                                                                          | no       | Delivered only when the input region is not `None`.                                                                                                  |
| `screen_edge() -> Option`                                                                                  | no       | Opt in to edge swipe-reveal (`Top` or `Bottom`); `None` is a normal tick-driven overlay.                                                             |
| `on_reveal`                                                                                                | no       | Called once per reveal of an armed edge, before its first frame.                                                                                     |
| `uses_settings() -> bool`                                                                                  | no       | Whether to bind `deck_settings_v1`; gates the settings hooks below.                                                                                  |
| `on_brightness` / `on_volume` / `on_night_mode` / `on_wifi_ap` / `on_restart_declined` / `on_capabilities` | no       | Settings events from the compositor, delivered before `tick`.                                                                                        |
| `on_preempted(active)`                                                                                     | no       | A modal full-screen overlay took (or released) the screen; delivered over `deck_settings_v1`, before `tick`. A transient overlay retracts on `true`. |
| `drain_settings_requests`                                                                                  | no       | Control requests to send this pass, drained after `render`.                                                                                          |
| `uses_alarm() -> bool`                                                                                     | no       | Whether to bind `deck_alarm_v1`; gates the alarm hooks below.                                                                                        |
| `on_alarm_ring(time, label, snooze_allowed)` / `on_alarm_stop`                                             | no       | Firing-alarm events from the compositor, delivered before `tick`.                                                                                    |
| `drain_alarm_requests`                                                                                     | no       | Alarm control requests (dismiss/snooze) to send this pass, drained after `render`.                                                                   |
| `uses_upgrade() -> bool`                                                                                   | no       | Whether to bind `deck_upgrade_v1`; gates the upgrade hook below.                                                                                     |
| `on_upgrade_state(snapshot)`                                                                               | no       | A complete upgrade snapshot, delivered before `tick` once the framework has seen `snapshot_done`. There is no return path.                           |
| `wants_cached_blit(now)` / `take_content_dirty` / `mark_content_dirty`                                     | no       | Hooks for the blit-only reveal animation (see below).                                                                                                |

`TickOutcome` carries three fields: `visible` (want to be on-screen — when `false` the framework unmaps the surface and
frees its buffers), `wants_render` (content changed; ignored while `!visible`), and `next_wake` (earliest instant to
tick again; `None` means "only on external events").

`LayerConfig` has two constructors that cover the common shapes: `LayerConfig::fullscreen(namespace)` (all four anchors,
`Layer::Top`, full input) and `LayerConfig::bottom_right(namespace, size)` (`Layer::Bottom`, bottom-right anchor, **no**
input region so passive corner content does not eat touches). The settings tray builds a `LayerConfig` by hand because
it wants `Layer::Overlay`, and the package-upgrade card because it wants a bottom-right surface on `Bottom` rather than
`Background`. `InputRegion` is just `Full` or `None`; the layer-shell default (whole surface accepts input) is the wrong
default for a passive indicator, so it is always set explicitly.

The trait re-exports the layer-shell `Layer` and `Anchor` enums from the crate root, so an overlay never depends on the
`wayland-protocols-wlr` crate directly.

## Rendering: the declarative tree

Overlays draw with the same `bmc-render` tree pipeline the WASM widgets use — `col`/`row`/`text`/`button`/`canvas`, laid
out host-side with Taffy and drawn with femtovg — but build the `TreeNode` structs directly in Rust, with no WASM
serialization step. `TreeUi` (`src/tree.rs`) holds the persistent interaction, animation, and layout caches so an
overlay can call `TreeUi::render(node, size, delta_ms, renderer)` each frame without threading that state by hand. Touch
is fed in with `push_touch`; the returned `TreeResult` reports slider drags by `touch_key` and `is_pressed(key)` backs
the hold-to-confirm buttons (one-frame latency, matching the WASM host).

The `canvas` immediate-mode element remains the escape hatch for gesture-driven or custom visuals.

## The hosted driver

`HostedOverlay` (`src/hosted.rs`) wraps one `SystemOverlay` for execution inside another process. It owns the overlay's
Wayland connection and export buffers but borrows the host's renderer and GPU stack for the actual frame, which the host
orchestrates. `HostedOverlay::connect` opens the layer-shell client, allocates the render target from the host's EGL
context, runs `init`, and arms the screen edge if the overlay opted in.

Each host loop pass touches every overlay in a fixed order (`bmc-wasm-host/src/main_loop.rs`):

1. `dispatch(egl)` — drain Wayland events: deliver touch, pick up reveal/hide, settings events (including preemption),
   and alarm ring/stop events, translate `wl_buffer.release` into freed export slots, and react to a configure-driven
   resize.
2. `tick(now)` — run the overlay's background work and recompute visibility. For a screen-edge overlay, visible means
   *revealed by the compositor* **and** the overlay's own `tick` says it wants to be on screen.
3. `needs_hide()` → `hide(egl)` — if mapped but no longer visible, attach a NULL buffer, round-trip the unmap, free the
   export buffers, and (for an edge overlay) re-arm the edge.
4. `needs_render(now)` → `render_hosted_overlay(...)` — if it should draw this pass, render through the shared renderer
   and submit to the overlay's own layer surface.
5. `forward_settings_requests()` / `forward_alarm_requests()` — drain and send any `deck_settings_v1` / `deck_alarm_v1`
   requests the overlay produced (a brightness drag or a Stop/Snooze tap is read during `render`, so this runs *after*
   render so the request goes out the same pass).

A failed or disconnected overlay is shut down (freeing its GPU resources) and dropped from the list; a `retain` would
skip `shutdown(egl)` and leak, because the render target does not free on `Drop`.

### Render and wake gates

The render decision is pure logic, unit-tested in `overlay.rs`. `overlay_needs_render` requires: not failed, visible,
the overlay wants a frame (a first show — visible but not yet mapped — always renders even without `wants_render`), the
8 ms inter-frame floor (`MIN_INTER_FRAME`) has elapsed, the layer-shell client is running, and a free export slot is
available. `overlay_needs_hide` is simply *mapped and not visible*.

The poll timeout (`overlay_poll_timeout`) must agree with the render gate: while invisible or otherwise non-rendering, a
latched `wants_render` must **not** request an immediate wake, or the host busy-spins on a frame that never renders. A
renderable overlay polls immediately (`Duration::ZERO`); a throttled one waits out the frame floor; otherwise the host
sleeps until the tick-requested `next_wake`.

## Buffers while hidden

A hidden overlay must not hold a full-screen buffer. Hiding is a two-step commit: attach a NULL buffer (which unmaps the
surface) and free the export DMA-BUFs. `OverlayRenderTarget::free_for_hide` (`src/gpu.rs`) tears down the GBM/GL buffers
and the cached `wl_buffer`s but keeps the target reusable — a later `ensure_current` reallocates lazily. This is
distinct from `destroy`, which is terminal and called only at shutdown.

The ordering in `hide` is load-bearing: the NULL attach is flushed *before* the exported buffers are destroyed, so the
compositor observes the unmap first. After a NULL-buffer unmap, layer-shell state is reset, so before the next real
buffer attach the client reapplies its layer/anchor/size/margins/input-region state via `ensure_ready_for_buffer_attach`
(`src/surface.rs`). Some compositor flows report a placeholder configure while unmapped (observed as a `1×200`
full-width top strip); that event is drained for protocol correctness but must not resize the reusable render target.

The compositor must reclaim its side too — drop the imported texture and release the buffer on the NULL-buffer unmap.
That half is in [`compositor-integration.md`](compositor-integration.md).

## GPU fence discipline

The GPU is an etnaviv Vivante part on MMUv2 with per-context address spaces; cross-context in-flight job interleaving
causes scene-freeze MMU faults, worked around by the cross-process lock at `/run/bmc-gpu-render.lock` and by waiting on
a GL fence before handing a buffer to the compositor. Overlays fit the existing discipline rather than open a new hole:

- **Hosted** overlays render inside the host's existing GL context and render loop. They add no new GPU context and ride
  the host's serialization and fence handoff — an overlay's buffer is published under the same GL-fence-before-handoff
  rule as a widget's. `render_hosted_overlay` stages the frame under the GPU lock, then exports.
- **Standalone** overlays are separate processes with their own GL context — exactly the cross-context client the lock
  exists for. `run_standalone` acquires `/run/bmc-gpu-render.lock` (via `GpuRenderLock::from_env`) and `wait_for_gpu`
  waits on an EGL fence (falling back to `glFinish`) around the submit.

`OverlayRenderTarget` is a double-buffered DMA-BUF target with `wl_buffer.release` tracking: it pairs two
lazily-allocated export buffers with a cache of minted `wl_buffer`s so a compositor release frees the matching slot for
reuse. It exports `ExportFormat::Alpha` (transparent overlays composite over the live scene), with depth disabled.

## Blit-only reveal animation

A reveal/dismiss slide must **not** re-lay-out and re-paint the panel every animation frame — too expensive to hit frame
rate on this GPU. The animation translates an already-rendered image instead. `OverlayRenderTarget` keeps a
`panel_cache`: a once-painted GL texture of the panel at its final layout.

- On a full paint, if `take_content_dirty()` reports the content changed, the host captures the just-painted band into
  the cache (`capture_panel`, a GPU→GPU shader copy, no CPU read-back).
- While `wants_cached_blit(now)` returns an offset and the cache exists, the host skips Taffy layout and femtovg paint
  entirely: it clears the export buffer transparent and copies the cached panel in at the current Y offset
  (`blit_cached_panel`), then fences/exports/attaches through the normal hosted path. An animation frame is *clear +
  blit + submit*.

The cache is re-rendered only when content changes (brightness value, WiFi-AP state, hostname/IP refresh, button/FSM
state) and is freed on hide so no full-screen allocation survives an unmap. Overlays that never animate leave all three
hooks at their defaults and always full-paint.

## Connectivity prober

`connectivity.rs` runs one process-global background thread (`connectivity-prober`, 128 KiB stack, spawned on first use)
that probes network state once per second — one `getifaddrs(3)` walk, one `uci -q show wireless` spawn, one
`/proc/net/wireless` read — and publishes a `Snapshot { ipv4, station_ssid, wifi_signal_dbm }`. Overlays read it with
`snapshot_if_changed(seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot>`: the prober bumps an internal version
only when the published content actually differs, and a reader passing back the version it last folded in gets `None` —
a single lock-free atomic load, no lock, no allocation — while nothing changed, so the read is safe on a per-frame
animation tick. The changed case is a value-swap mutex read that never blocks, so a WiFi-driver stall holding the
kernel's rtnl lock can no longer freeze the host render loop. `None` with `seen: None` means the prober has not
published yet; `Snapshot { ipv4: None, .. }` means genuinely offline.

"Online" is defined as *at least one non-loopback interface holds a routable IPv4 address* (loopback, link-local
`169.254/16`, and unspecified excluded); the device is WiFi-centric and there is no separate ethernet-carrier probe. The
IPv4 pick prefers WiFi station interfaces (kernel `wlan*` prefix, since the trailing index is not stable across boots),
sorts AP-mode interfaces last so a coexisting setup AP does not shadow the real uplink, then falls back to lexicographic
name order for determinism. The station SSID comes from OpenWrt's `uci` CLI; the probe is purely observational and never
starts, retries, or reconfigures WiFi.

## Adding an overlay

1. Create a crate under `system-overlays/` with a type implementing `SystemOverlay` and a `main.rs` that calls
   `bmc_system_overlay::run_standalone(Box::new(...))`.
2. To host it in `bmc-wasm-host`: add it as an optional dependency behind a Cargo feature, add the feature to `default`,
   and add one `register_overlay!` line in `build_overlays` (`bmc-wasm-host/src/overlays.rs`). That single line is the
   source of truth for the feature gate, the runtime name (env-var lookup and logging), and the constructor.

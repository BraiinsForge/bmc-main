# System overlays 1b — framework crate + standalone entrypoint — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the `bmc-system-overlay` framework crate — a self-contained `wlr-layer-shell` client that renders
against `bmc-render`'s `dyn Renderer` — and a standalone entrypoint that owns its connection, renderer, and loop. Prove
it with a throwaway validation overlay run as its own process against the plan-1a compositor.

**Architecture:** The framework depends on `bmc-widget` (for `EglContext`, `SharedRenderScratch`, `DoubleBufferState`,
`DmaBufInfo`, and the `create_buffer_from_dmabuf` helper), `bmc-render` (for `FemtoVgRenderer` + the `Renderer` trait +
the `TreeNode`/`layout_and_render` tree pipeline), and `bmc-gpu-render-lock`. It does **not** depend on `bmc-wasm-host`
(the host depends on the framework in plan 1c). The standalone entrypoint orchestrates a frame exactly like
`WidgetSlot::render` does — begin staging FBO, draw via the renderer, blit to an export DMA-BUF, GL-fence wait,
export+swap, mint a `wl_buffer`, attach to the layer surface, commit.

**Tech Stack:** Rust, `wayland-client`, `wayland-protocols-wlr` (layer-shell client), `bmc-widget` (gpu feature),
`bmc-render`, `bmc-gpu-render-lock`, EGL/GBM/GLES. Runs on host (dev) and ARM (device).

**Context the executor needs:**

- Sub-plan 1b of BDK-416. Spec: `docs/superpowers/specs/2026-06-07-system-overlays-design.md`. Depends on plan 1a
  (compositor must advertise `zwlr_layer_shell_v1`).
- Mirror the existing client at `bmc-widget/src/surface/deck_widget.rs` + helpers in `bmc-widget/src/surface/common.rs`.
  The layer client is that client with `widget_manager.get_widget_surface` replaced by `layer_shell.get_layer_surface`
  plus layer configuration, and the deck_widget configure handshake replaced by the layer-surface `Configure`/`Closed`
  events.
- Mirror the render→export pipeline in `bmc-wasm-host/src/slot.rs::render` (begin_frame → with renderer → flush → blit →
  flush_and_wait_gl → export_and_swap → mint+attach).
- Run cargo sandboxed; `nix fmt` (plain) before each commit; no ticket IDs in code comments; `#[expect]` not `#[allow]`.

---

## File Structure

- Create `system-overlays/bmc-system-overlay/Cargo.toml` — crate manifest.
- Create `system-overlays/bmc-system-overlay/src/lib.rs` — crate root, re-exports.
- Create `.../src/overlay.rs` — the `SystemOverlay` trait, `LayerConfig`, `Anchor`/`Layer`/`InputRegion` re-exports,
  `TouchEvent`, `TickOutcome`.
- Create `.../src/surface.rs` — `LayerSurfaceClient` (the layer-shell Wayland client; mirrors deck_widget).
- Create `.../src/gpu.rs` — `wait_for_gpu` (GL-fence wait, ported from host) + `OverlayRenderTarget` (double-buffer +
  `wl_buffer` cache + release tracking).
- Create `.../src/standalone.rs` — `run_standalone(overlay: Box<dyn SystemOverlay>)` entrypoint (config and size are
  derived from the overlay/compositor internally).
- Create `.../src/tree.rs` — a thin `TreeUi` helper holding the `ProcessContext` state (Taffy tree +
  interaction/animation maps) so an overlay can render a `TreeNode` against a `&mut dyn Renderer`.
- Create `system-overlays/validation-overlay/Cargo.toml` + `src/main.rs` — throwaway standalone binary using the
  framework.
- Modify root `Cargo.toml` members.

---

## Task 1: Scaffold the framework crate

**Files:**

- Create: `system-overlays/bmc-system-overlay/Cargo.toml`, `system-overlays/bmc-system-overlay/src/lib.rs`

- Modify: root `Cargo.toml`

- [ ] **Step 1: Manifest**

`system-overlays/bmc-system-overlay/Cargo.toml`:

```toml
[package]
name = "bmc-system-overlay"
version = "0.1.0"
edition = "2024"
authors = ["Braiins Systems s.r.o."]
description = "Framework for privileged wlr-layer-shell system overlays"

[dependencies]
bmc-widget = { workspace = true, features = ["gpu"] }
bmc-render.workspace = true
bmc-gpu-render-lock.workspace = true
taffy.workspace = true
glow.workspace = true
wayland-client.workspace = true
wayland-protocols.workspace = true
wayland-protocols-wlr = { workspace = true, features = ["client"] }
wayland-backend.workspace = true
anyhow.workspace = true
tracing.workspace = true
libc.workspace = true

[lints]
workspace = true
```

Add the two genuinely-missing entries to root `Cargo.toml` `[workspace.dependencies]`:

- `bmc-render = { path = "bmc-render" }` (confirm the path against the other `bmc-*` workspace entries).
- `wayland-protocols-wlr = "0.3.12"` (matches the lockfile; currently only transitive via smithay).

Already present — **reuse, do not re-add** (re-adding duplicates the key and breaks the manifest): `bmc-gpu-render-lock`
(Cargo.toml:147), `bmc-widget`, `wayland-client`, `wayland-protocols`, `wayland-backend`, `taffy`, `glow`, `anyhow`,
`tracing`, `libc`.

- [ ] **Step 2: Crate root (minimal — modules are added by the task that creates each file)**

`system-overlays/bmc-system-overlay/src/lib.rs` starts with only what compiles standalone (the protocol enum re-exports
depend on no local module):

```rust
//! Framework for privileged system overlays rendered as wlr-layer-shell clients.

// Re-export the layer-shell client enums so overlays can build a LayerConfig
// without importing the protocol crate directly.
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;
```

Each later task appends its own `mod` + `pub use` line(s) **in the same commit that creates the module file**, so every
commit compiles (Task 2 → `mod overlay`, Task 3 → `mod surface`, Task 4 → `mod gpu; mod tree` + their re-exports, Task 5
→ `mod standalone`). Never commit a `mod`/`pub use` for a file that does not exist yet.

- [ ] **Step 3: Add to workspace**

Add `"system-overlays/bmc-system-overlay"` to root `Cargo.toml` `members`.

- [ ] **Step 4: Build**

Run: `nix develop -c cargo build -p bmc-system-overlay` Expected: PASS — the crate compiles with just the enum
re-exports, confirming dependency resolution before any module lands.

---

## Task 2: The `SystemOverlay` trait and config types

**Files:**

- Create: `system-overlays/bmc-system-overlay/src/overlay.rs`

- Test: same file

- [ ] **Step 1: Write the trait and config (complete)**

`src/overlay.rs`:

```rust
use std::time::Instant;

use bmc_render::renderer::Renderer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;

/// A touch event delivered to an overlay (logical coordinates within the surface).
#[derive(Debug, Clone, Copy)]
pub enum TouchEvent {
    Down { id: i32, x: f64, y: f64 },
    Motion { id: i32, x: f64, y: f64 },
    Up { id: i32 },
    Cancel,
}

/// What region of the surface accepts touch input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRegion {
    /// Whole surface accepts input (the layer-shell default).
    Full,
    /// Surface accepts no input; touches fall through to what is behind it.
    None,
}

/// Static layer-surface configuration, applied once at map time.
#[derive(Debug, Clone)]
pub struct LayerConfig {
    pub layer: Layer,
    pub anchor: Anchor,
    /// Requested size in logical pixels. A zero axis with both opposite anchors
    /// set asks the compositor to stretch that axis.
    pub size: (u32, u32),
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    /// Pixels of output edge the surface reserves (layer-shell exclusive zone).
    /// `0` reserves nothing — correct for every overlay here (fullscreen
    /// blocker, passive corner indicator, top panel). Kept as a knob because
    /// the spec names it as framework plumbing.
    pub exclusive_zone: i32,
    pub namespace: String,
    pub input: InputRegion,
}

impl LayerConfig {
    /// A fullscreen overlay anchored to all four edges.
    #[must_use]
    pub fn fullscreen(namespace: impl Into<String>) -> Self {
        Self {
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 0),
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: namespace.into(),
            input: InputRegion::Full,
        }
    }
}

/// Result of an overlay's per-pass background work.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickOutcome {
    /// The overlay's content changed and it wants to be rendered this pass.
    pub wants_render: bool,
    /// Earliest instant the overlay wants to be ticked again (for non-event
    /// driven work such as a clock). `None` means "only on external events".
    pub next_wake: Option<Instant>,
}

/// A privileged system overlay. Implementors do background work in `tick`,
/// draw in `render`, and declare placement via `layer_config`.
pub trait SystemOverlay {
    /// Called once before the first render.
    fn init(&mut self) {}

    /// Static placement and input policy.
    fn layer_config(&self) -> LayerConfig;

    /// Per-pass background work. Return whether a render is wanted and when to
    /// wake next. Must not block.
    fn tick(&mut self, now: Instant) -> TickOutcome;

    /// Draw the overlay. `size` is the surface size in logical pixels. The
    /// `&mut dyn Renderer` is valid only for this call: do not store it.
    fn render(&mut self, renderer: &mut dyn Renderer, size: (u32, u32));

    /// Handle a touch event (only delivered when input region is not `None`).
    fn on_touch(&mut self, _event: TouchEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_config_anchors_all_edges() {
        let c = LayerConfig::fullscreen("test");
        assert!(c.anchor.contains(Anchor::Top));
        assert!(c.anchor.contains(Anchor::Bottom));
        assert!(c.anchor.contains(Anchor::Left));
        assert!(c.anchor.contains(Anchor::Right));
        assert_eq!(c.size, (0, 0));
        assert_eq!(c.input, InputRegion::Full);
    }
}
```

- [ ] **Step 2: Append the module to `lib.rs`**

Add to `lib.rs` (same commit): `mod overlay;` and
`pub use overlay::{InputRegion, LayerConfig, SystemOverlay, TickOutcome, TouchEvent};`. The crate now compiles with
`overlay` present.

- [ ] **Step 3: Run the test**

Run: `nix develop -c cargo test -p bmc-system-overlay overlay::tests` Expected: PASS (1 test). No comment-out dance is
needed — `lib.rs` only references modules whose files exist.

- [ ] **Step 4: Commit**

```bash
nix fmt
git add system-overlays/bmc-system-overlay Cargo.toml
git commit -F - <<'EOF'
system-overlays: Add overlay framework crate and trait #BDK-416

- scaffold bmc-system-overlay crate (compiles at every commit)
- define the SystemOverlay trait, LayerConfig (with exclusive_zone),
  InputRegion, TouchEvent, and TickOutcome
EOF
```

---

## Task 3: The layer-shell Wayland client

**Files:**

- Create: `system-overlays/bmc-system-overlay/src/surface.rs`

This mirrors `bmc-widget/src/surface/deck_widget.rs` and reuses `bmc-widget/src/surface/common.rs` helpers. Open both
files and replicate the structure; the deltas below are the only behavioral differences.

- [ ] **Step 1: Define the client state and struct**

`src/surface.rs` — model on `DeckWidgetSurfaceState`/`DeckWidgetSurfaceClient` (deck_widget.rs:113-243), with these
global bindings: `wl_compositor`, `zwlr_layer_shell_v1`, `zwp_linux_dmabuf_v1`, `wl_seat`. Replace the
`widget_manager`/`widget_surface` fields with:

```rust
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    /// Set true on the first layer-surface Configure (after which we may map).
    configured: bool,
    /// Compositor-suggested size from the latest Configure.
    configured_size: (u32, u32),
    pending_touch: Vec<crate::overlay::TouchEvent>,
    /// Surface-dirty from a Configure/resize only. Overlays do not use
    /// compositor frame callbacks; redraw pacing is the framework's
    /// tick/next_wake job, so no wl_surface.frame is ever requested.
    needs_render: bool,
```

Imports:

```rust
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_region, wl_registry, wl_seat, wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use bmc_widget::egl::DmaBufInfo;
use bmc_widget::{create_buffer_from_dmabuf, submit_buffer_to_surface, drain_released_buffers,
    poll_dispatch, BufferSlotMap, ReleasedBuffer, ReleasedBufferSet};
```

**Visibility (do this in a small preliminary commit to `bmc-widget`):** the reuse-seam items are not reachable from a
separate crate — the `surface::common` and `poll` modules are private (`surface.rs:10`, `lib.rs:5`) and the helpers are
`pub(crate)`. Fix by (1) marking each item `pub` and (2) **re-exporting them at the `bmc-widget` crate root** so the
private modules need not be opened:

```rust
// bmc-widget/src/lib.rs
// `mod poll;` is NOT feature-gated, so this stays ungated.
pub use poll::poll_dispatch;                 // free helper, currently bmc_widget::poll::poll_dispatch (poll.rs:46)

// `pub mod surface` is `#[cfg(feature = "gpu")]` (lib.rs:6) and non-GPU
// consumers exist (`bmc`, `widgets/digital-clock`), so the surface::common
// re-exports MUST carry the same gate or they break the default build.
#[cfg(feature = "gpu")]
pub use surface::common::{
    create_buffer_from_dmabuf, submit_buffer_to_surface,
    ReleasedBuffer, ReleasedBufferSet, BufferSlotMap, drain_released_buffers, drain_released_buffer_slots,
};
```

`DmaBufInfo`, `DoubleBufferState`, and `SlotReleaseState` live in `bmc_widget::egl`, which is itself
`#[cfg(feature = "gpu")]` and `pub` — just make those three items `pub` there (used by the Task 4 render target).
`bmc-system-overlay` enables `bmc-widget`'s `gpu` feature, so the gated items are available to it. After this, the
framework imports everything as `bmc_widget::<Item>` (crate root) or `bmc_widget::egl::<Item>`. Note: `bmc-widget`'s
`impl_common_dispatch!` macro covers `wl_compositor`/`wl_surface`/`wl_callback`/dmabuf/params but **not** `wl_buffer`,
so write the `wl_buffer` Dispatch inline (Step 5).

- [ ] **Step 2: Registry bind (delta from deck_widget.rs:771-822)**

Copy the `Dispatch<wl_registry, ()>` impl, replacing the `"deck_widget_manager_v1"` arm with:

```rust
                "zwlr_layer_shell_v1" => {
                    let layer_shell = registry
                        .bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(name, version.min(4), qh, ());
                    state.layer_shell = Some(layer_shell);
                }
```

Keep the `wl_compositor`, `zwp_linux_dmabuf_v1`, and `wl_seat` arms unchanged.

- [ ] **Step 3: Connect + create the layer surface + apply config**

Model on `DeckWidgetSurfaceClient::connect` (deck_widget.rs:393), which already does `Connection::connect_to_env()` (the
overlay is its own client) — reuse its registry-roundtrip/configure-wait shape rather than the fd variant. After binding
globals, create the surface and layer surface and apply config:

```rust
    pub fn connect(config: &crate::overlay::LayerConfig) -> anyhow::Result<Self> {
        let conn = Connection::connect_to_env()
            .map_err(|e| anyhow::anyhow!("wayland connect: {e}"))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());

        let mut state = State::default(); // derive Default or construct explicitly
        queue.roundtrip(&mut state).map_err(|e| anyhow::anyhow!("roundtrip: {e}"))?;

        let compositor = state.compositor.clone().context("wl_compositor missing")?;
        let layer_shell = state.layer_shell.clone().context("zwlr_layer_shell_v1 missing")?;
        anyhow::ensure!(state.linux_dmabuf.is_some(), "zwp_linux_dmabuf_v1 missing");

        let surface = compositor.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None, // default output
            config.layer,
            config.namespace.clone(),
            &qh,
            (),
        );
        layer_surface.set_anchor(config.anchor);
        layer_surface.set_size(config.size.0, config.size.1);
        layer_surface.set_margin(
            config.margin_top, config.margin_right, config.margin_bottom, config.margin_left,
        );
        layer_surface.set_exclusive_zone(config.exclusive_zone);
        if matches!(config.input, crate::overlay::InputRegion::None) {
            // Empty input region: the surface accepts no touches.
            let region = compositor.create_region(&qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();
        }
        surface.commit(); // triggers the initial Configure

        state.surface = Some(surface);
        state.layer_surface = Some(layer_surface);

        // Wait for the first Configure.
        while !state.configured {
            queue.blocking_dispatch(&mut state)
                .map_err(|e| anyhow::anyhow!("dispatch awaiting configure: {e}"))?;
        }

        Ok(Self { conn, queue, state })
    }
```

- [ ] **Step 4: Layer-surface event handling (the key delta)**

Implement `Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()>`:

```rust
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer_surface.ack_configure(serial);
                state.configured_size = (width, height);
                state.configured = true;
                state.needs_render = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.running = false;
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 5: Buffer mint + attach + touch (reuse common.rs)**

Expose two buffer methods mirroring `DeckWidgetSurfaceClient`, so the render target (Task 4) owns the per-slot
`wl_buffer` cache and release mapping:

- `mint_wl_buffer(&mut self, info: &DmaBufInfo, slot: usize) -> Result<wl_buffer::WlBuffer>` —
  `create_buffer_from_dmabuf` then `self.state.buffer_slots.insert(buffer.id(), slot)` (mirror deck_widget.rs:618-626).
- `submit_buffer_with_wl_buffer(&mut self, info: &DmaBufInfo, buffer: &wl_buffer::WlBuffer) -> Result<()>` —
  `surface.attach`, `damage_buffer`, `surface.commit()` via `submit_buffer_to_surface` (common.rs:182-199) with
  `request_frame: false` (mirror deck_widget.rs:585-592).

The overlay does **not** request `wl_surface.frame`: 1a does not deliver layer frame callbacks, and redraw pacing is the
framework's `tick`/`next_wake` job.

Implement the `Dispatch` impls for `wl_buffer`, `wl_callback`, `zwp_linux_buffer_params_v1`, `wl_compositor`,
`wl_surface`, `wl_region`, `zwlr_layer_shell_v1`, `wl_seat`, and `wl_touch`:

- `wl_buffer` — handle `Release` exactly like deck_widget.rs:1047-1050: `state.released_buffers.insert(buffer_id)`.
  **This is required** — it is how an export slot becomes reusable. The double buffer stalls after 2 frames without it.
- `wl_callback` — no-op (no frame is ever requested, so `Done` never arrives).
- `wl_seat`/`wl_touch` — mirror deck_widget.rs:1056-1125 (acquire `wl_touch` on capability; push events to
  `pending_touch`).
- the rest — no-op `()`.

Reuse `ReleasedBufferSet`, `BufferSlotMap`, and `drain_released_buffers`/`drain_released_buffer_slots` from `bmc-widget`
(deck_widget.rs:27-28 imports them; widen visibility if needed). Add the `released_buffers: ReleasedBufferSet` and
`buffer_slots: BufferSlotMap` fields to the client state, mirroring deck_widget. `needs_render` is set only by the
`Configure` handler (resize), never by a frame callback.

Expose helpers the entrypoint needs:

```rust
    pub fn size(&self) -> (u32, u32) { self.state.configured_size }
    pub fn running(&self) -> bool { self.state.running }
    pub fn take_needs_render(&mut self) -> bool { std::mem::take(&mut self.state.needs_render) }
    pub fn drain_touch(&mut self) -> Vec<crate::overlay::TouchEvent> {
        std::mem::take(&mut self.state.pending_touch)
    }
    /// Buffers the compositor has released since last call, so the render
    /// target can mark their export slots reusable. Mirrors
    /// `DeckWidgetSurfaceClient::drain_released_buffers` (deck_widget.rs:644).
    pub fn drain_released_buffers(&mut self) -> Vec<ReleasedBuffer> {
        drain_released_buffers(&self.state.buffer_slots, &mut self.state.released_buffers)
    }
    /// Flush, then block up to `timeout_ms` (-1 = forever) running the full
    /// `prepare_read -> poll -> read/cancel_read -> dispatch_pending` sequence,
    /// reusing `bmc-widget`'s poll helper (poll.rs:46, as deck_widget's
    /// `connect` does). A bare `poll(2)` + `dispatch_pending` would never read
    /// the fd and miss events — do not roll your own.
    pub fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<()> {
        self.conn.flush().ok();
        // `bmc_widget::poll_dispatch` (free helper at poll.rs:46, re-exported at
        // the crate root in the Step 1 visibility commit) does prepare_read ->
        // poll -> read/cancel -> dispatch_pending.
        poll_dispatch(&self.conn, &mut self.queue, &mut self.state, timeout_ms)
            .map(|_outcome| ())
            .map_err(|e| anyhow::anyhow!("poll_dispatch: {e}"))
    }
    pub fn connection_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;
        self.conn.as_fd().as_raw_fd()
    }
    pub fn flush(&self) -> anyhow::Result<()> {
        self.conn.flush().map_err(|e| anyhow::anyhow!("wl flush: {e}"))
    }
    /// Linux-dmabuf global + queue handle for minting buffers.
    pub(crate) fn dmabuf(&self) -> &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 {
        self.state.linux_dmabuf.as_ref().expect("BUG: dmabuf checked at connect")
    }
```

- [ ] **Step 6: Build**

Run: `nix develop -c cargo build -p bmc-system-overlay` Expected: PASS (with the standalone/gpu/tree modules still
stubbed enough to compile, or temporarily commented in `lib.rs`).

- [ ] **Step 7: Commit**

```bash
nix fmt
git add system-overlays/bmc-system-overlay/src/surface.rs bmc-widget
git commit -F - <<'EOF'
system-overlays: Add layer-shell surface client #BDK-416

- mirror the deck_widget Wayland client for wlr-layer-shell: bind the
  layer-shell global, create a configured layer surface, ack configure,
  and mint/attach dmabuf buffers via the shared bmc-widget helpers
- surface touch events to the framework (no frame callbacks: overlays self-pace)
EOF
```

---

## Task 4: GPU frame helper and standalone orchestration

**Files:**

- Create: `system-overlays/bmc-system-overlay/src/gpu.rs`

- Create: `system-overlays/bmc-system-overlay/src/tree.rs`

- [ ] **Step 1: Port the GL-fence wait**

`src/gpu.rs` — port `flush_and_wait_gl` + the GL fence wait from `bmc-wasm-host/src/host.rs:73-145` (it uses `egl.gl()`
glow calls). Provide:

```rust
use bmc_widget::egl::EglContext;

/// Stall the CPU until the GPU has finished the submitted commands, so the
/// exported DMA-BUF is safe to hand to the compositor. Mirrors the host's
/// flush_and_wait_gl. Uses a GL fence sync when available, else glFinish.
pub fn wait_for_gpu(egl: &EglContext) {
    // Port of bmc-wasm-host host.rs wait_for_gl_fence with a glFinish fallback.
    // ... (copy the GlFenceSync path; fall back to egl.gl().finish()).
}
```

This duplicates host logic; note in the commit body that hoisting it into `bmc-widget` is a future cleanup so host and
framework share one copy.

- [ ] **Step 2: `TreeUi` helper**

`src/tree.rs` — hold the `ProcessContext` backing state so an overlay can render a `TreeNode`:

```rust
use std::collections::HashMap;

use bmc_render::renderer::Renderer;
use bmc_render::tree::{layout_and_render, NodeContext, ProcessContext, TreeNode, TreeResult};
use bmc_render::{AnimationState, FrameTimings, ModalState, ScrollState, TransitionState, TransitionStateKey};
use bmc_render::interaction::InteractionState;
use taffy::prelude::TaffyTree;

#[derive(Default)]
pub struct TreeUi {
    interaction: InteractionState,
    modal_states: HashMap<String, ModalState>,
    scroll_states: HashMap<String, ScrollState>,
    animation_states: HashMap<u64, AnimationState>,
    transition_states: HashMap<TransitionStateKey, TransitionState>,
    taffy: TaffyTree<NodeContext>,
    frame_counter: u64,
}

impl TreeUi {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Lay out and draw `node` at `size` against `renderer`.
    pub fn render(
        &mut self,
        node: &TreeNode,
        size: (u32, u32),
        delta_ms: u32,
        renderer: &mut dyn Renderer,
    ) -> anyhow::Result<TreeResult> {
        self.frame_counter = self.frame_counter.wrapping_add(1);
        let mut timings = bmc_render::FrameTimings::default();
        let mut ctx = ProcessContext {
            interaction: &mut self.interaction,
            modal_states: &mut self.modal_states,
            scroll_states: &mut self.scroll_states,
            animation_states: &mut self.animation_states,
            transition_states: &mut self.transition_states,
            taffy: &mut self.taffy,
            frame_counter: self.frame_counter,
            delta_ms,
        };
        let (result, _has_active) = layout_and_render(
            node, size.0 as f32, size.1 as f32, renderer, &mut timings, &mut ctx,
        )?;
        Ok(result)
    }
}
```

Resolved paths (verified):
`ModalState`/`ScrollState`/`AnimationState`/`TransitionState`/`TransitionStateKey`/`FrameTimings` are re-exported at the
`bmc_render` crate root. `InteractionState` is **not** — use `bmc_render::interaction::InteractionState`. `NodeContext`
is `bmc_render::tree::NodeContext`. `TaffyTree` comes from `taffy::prelude` (not `bmc-render`), so add `taffy` to the
crate's dependencies (it is already a workspace dep used by `bmc-render`; `taffy.workspace = true`).

- [ ] **Step 3: `OverlayRenderTarget` (double buffer + release tracking)**

Add to `gpu.rs` (and `pub use gpu::OverlayRenderTarget` from `lib.rs`) a port of `bmc-wasm-host/src/render_target.rs`'s
`EglRenderTarget` — the same combination of `DoubleBufferState` + a `[Option<wl_buffer::WlBuffer>; 2]` cache +
`SlotReleaseState` (all from `bmc-widget::egl`). This is what makes the compositor's `wl_buffer.release` actually free
an export slot; without it the double buffer stalls after two frames. Mirror the struct + release methods at
render_target.rs:28-113 and the constructor `new_egl` at render_target.rs:166:

```rust
use bmc_widget::egl::{DoubleBufferState, DmaBufInfo, EglContext, SlotReleaseState};
use bmc_widget::ReleasedBuffer;
use wayland_client::protocol::wl_buffer;

pub struct OverlayRenderTarget {
    buffers: DoubleBufferState,
    wl_buffers: [Option<wl_buffer::WlBuffer>; 2],
    release: SlotReleaseState,
}

impl OverlayRenderTarget {
    pub fn new(egl: &EglContext, w: u32, h: u32) -> anyhow::Result<Self>;
    pub fn ensure_current(&mut self, egl: &EglContext) -> anyhow::Result<()>;
    pub fn current_fbo(&self) -> glow::Framebuffer; // buffers.current_ref().fbo
    pub fn export_and_swap(&mut self) -> anyhow::Result<(DmaBufInfo, usize)>;
    /// True when the next export slot's buffer has been released (or never used).
    pub fn available(&self) -> bool;
    pub fn mark_presented(&mut self, slot: usize); // release.mark_presented(slot)
    pub fn mark_released_buffer(&mut self, released: &ReleasedBuffer); // map id->slot via wl_buffers, release.mark_released(slot)
    /// Mint (once) and cache the wl_buffer for `slot` via the client, mirroring
    /// render_target.rs:95-113 `wl_buffer_for_slot`.
    pub fn wl_buffer_for_slot(
        &mut self,
        client: &mut crate::surface::LayerSurfaceClient,
        info: &DmaBufInfo,
        slot: usize,
    ) -> anyhow::Result<wl_buffer::WlBuffer>;
    /// Free GL/EGL/GBM resources. `DoubleBufferState` does NOT clean up on Drop
    /// (egl.rs:901) — the owner must call `destroy_all(egl)`. Also `.destroy()`
    /// each cached `wl_buffer`. The owner (standalone loop on exit; host on
    /// overlay-drop and shutdown) must call this or leak per overlay.
    pub fn destroy(&mut self, egl: &EglContext); // self.buffers.destroy_all(egl) + wl_buffers[*].take().destroy()
}
```

The inner `DoubleBufferState::new(width: u32, height: u32, depth: Depth)` takes no `EglContext` (egl.rs:803) and
allocates lazily in `ensure_current`; pass the same `Depth` variant the host's `EglRenderTarget` uses (egl.rs:60 —
confirm against render_target.rs). `OverlayRenderTarget::new`'s `egl` arg is for parity/future use; it may be unused.
These items (`SlotReleaseState`/`DoubleBufferState`/`ReleasedBuffer`) are made `pub` in the Task 3 Step 1 visibility
commit. This wrapper is a candidate to hoist into `bmc-widget` later so the host and the framework share one copy (note
as cleanup, do not refactor the host here).

- [ ] **Step 4: Build**

Run: `nix develop -c cargo build -p bmc-system-overlay` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
nix fmt
git add system-overlays/bmc-system-overlay/src/gpu.rs system-overlays/bmc-system-overlay/src/tree.rs system-overlays/bmc-system-overlay/src/lib.rs
git commit -F - <<'EOF'
system-overlays: Add GPU fence wait, tree helper, render target #BDK-416

- port the host GL-fence wait so an exported buffer is safe to hand off
- add TreeUi holding the layout/animation context to render a TreeNode
  against a dyn Renderer
- add OverlayRenderTarget (double buffer + wl_buffer cache + release
  tracking) so wl_buffer.release frees an export slot
EOF
```

---

## Task 5: The standalone entrypoint

**Files:**

- Create: `system-overlays/bmc-system-overlay/src/standalone.rs`

- [ ] **Step 1: Write `run_standalone` (complete orchestration)**

`src/standalone.rs`:

```rust
use std::time::{Duration, Instant};

use bmc_gpu_render_lock::GpuRenderLock;
use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer as _; // brings begin_frame/flush/fill_rect into scope
use bmc_widget::egl::{EglContext, SharedRenderScratch};

use crate::gpu::{wait_for_gpu, OverlayRenderTarget};
use crate::overlay::{LayerConfig, SystemOverlay};
use crate::surface::LayerSurfaceClient;

const MIN_INTER_FRAME: Duration = Duration::from_millis(8);

/// Run an overlay as its own process: own connection, renderer, and loop.
pub fn run_standalone(mut overlay: Box<dyn SystemOverlay>) -> anyhow::Result<()> {
    let config: LayerConfig = overlay.layer_config();

    // Connect and configure the layer surface; learn the real size.
    let mut client = LayerSurfaceClient::connect(&config)?;
    let (w, h) = client.size();
    let (w, h) = (if w == 0 { config.size.0.max(1) } else { w }, if h == 0 { config.size.1.max(1) } else { h });

    // GPU stack (owned in standalone mode).
    let egl = EglContext::new()?;
    let scratch = SharedRenderScratch::new(&egl, w, h)?;
    let gpu_lock = GpuRenderLock::from_env()?;
    let mut renderer = unsafe {
        FemtoVgRenderer::new(|s| EglContext::get_proc_address(s), w, h, scratch.staging_fbo_id(), 0)?
    };
    let mut target = OverlayRenderTarget::new(&egl, w, h)?;

    overlay.init();
    let mut last_render: Option<Instant> = None;
    // A wanted render that can't run yet (no free buffer slot, or inter-frame
    // throttle) must NOT be lost — it stays pending until a frame actually
    // renders. take_needs_render()/tick are consumed every pass, so we latch
    // the request here rather than re-reading it.
    let mut pending_render = false;

    while client.running() {
        // Drain what the previous poll_dispatch delivered.
        for ev in client.drain_touch() {
            overlay.on_touch(ev);
        }
        for released in client.drain_released_buffers() {
            target.mark_released_buffer(&released);
        }

        let now = Instant::now();
        let tick = overlay.tick(now);
        if tick.wants_render || client.take_needs_render() {
            pending_render = true;
        }

        // Remaining time on the inter-frame floor, if a render was throttled.
        let inter_frame_remaining = last_render
            .and_then(|t| MIN_INTER_FRAME.checked_sub(now.duration_since(t)))
            .filter(|d| !d.is_zero());

        // `target.available()` gates on a free (released) export slot, so we
        // never draw into a buffer the compositor is still displaying.
        if pending_render && target.available() && inter_frame_remaining.is_none() {
            render_frame(&egl, &scratch, &gpu_lock, &mut target, &mut renderer, &mut *overlay, &mut client, (w, h))?;
            pending_render = false;
            last_render = Some(now);
        }

        let timeout = if pending_render && inter_frame_remaining.is_some() {
            // Throttled: wake when the inter-frame floor expires.
            inter_frame_remaining
        } else if pending_render {
            // Wanted but blocked on a free buffer slot: the incoming
            // wl_buffer.release wakes poll_dispatch via the fd; no timer needed.
            None
        } else {
            tick.next_wake.map(|t| t.saturating_duration_since(now))
        };
        let timeout_ms = timeout.map_or(-1, |d| i32::try_from(d.as_millis().max(1)).unwrap_or(i32::MAX));
        // poll_dispatch runs the full prepare_read -> poll -> read -> dispatch
        // sequence (a bare poll(2)+dispatch_pending would miss the fd).
        client.poll_dispatch(timeout_ms)?;
    }
    // DoubleBufferState does not free on Drop; release GL/EGL/GBM explicitly.
    target.destroy(&egl);
    Ok(())
}

fn render_frame(
    egl: &EglContext,
    scratch: &SharedRenderScratch,
    gpu_lock: &GpuRenderLock,
    target: &mut OverlayRenderTarget,
    renderer: &mut FemtoVgRenderer,
    overlay: &mut dyn SystemOverlay,
    client: &mut LayerSurfaceClient,
    size: (u32, u32),
) -> anyhow::Result<()> {
    let _lock = gpu_lock.lock("system_overlay_standalone")?;
    target.ensure_current(egl)?;
    let _staging = scratch.begin_frame(egl, size.0, size.1);
    renderer.begin_frame(size.0, size.1, 1.0);
    // BOTH scratch.begin_frame and FemtoVgRenderer::begin_frame clear opaque
    // black (renderer.rs:1096). A see-through overlay must start transparent,
    // so re-clear the bound staging FBO to alpha 0 AFTER femtovg's clear and
    // before drawing — the recorded draws then flush over a transparent base.
    unsafe {
        use glow::HasContext as _;
        let gl = egl.gl();
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
    overlay.render(renderer, size);
    renderer.flush();
    scratch.blit_to(egl, target.current_fbo(), size.0, size.1);
    wait_for_gpu(egl);
    drop(_lock);
    let (dmabuf, slot) = target.export_and_swap()?;
    // Mint+cache the wl_buffer for this slot, then attach. mark_presented marks
    // the slot in-flight until the compositor sends wl_buffer.release.
    let wl_buffer = target.wl_buffer_for_slot(client, &dmabuf, slot)?;
    client.submit_buffer_with_wl_buffer(&dmabuf, &wl_buffer)?;
    client.flush()?;
    target.mark_presented(slot);
    Ok(())
}
```

Confirmed APIs (corrected from the host/lock sources, not "maybe"): `GpuRenderLock::lock(scope: &'static str)`
(bmc-gpu-render-lock/src/lib.rs:229; the compositor and host call it this way) — there is no `acquire`.
`GpuRenderLock::from_env()` is correct. `SharedRenderScratch::begin_frame`/`blit_to`, `FemtoVgRenderer::new` (5 args)
are correct. The `OverlayRenderTarget` methods (Task 4 Step 3) wrap `DoubleBufferState`/`SlotReleaseState`;
`current_fbo()` returns `current_ref().fbo`. `client.poll_dispatch` wraps `bmc-widget`'s poll helper (Task 3 Step 5) —
never a bare `dispatch_pending` with an external `poll`.

- [ ] **Step 2: Build**

Run: `nix develop -c cargo build -p bmc-system-overlay` Expected: PASS.

- [ ] **Step 3: Commit**

```bash
nix fmt
git add system-overlays/bmc-system-overlay/src/standalone.rs system-overlays/bmc-system-overlay/src/lib.rs
git commit -F - <<'EOF'
system-overlays: Add standalone overlay entrypoint #BDK-416

- run an overlay as its own process: own EGL/renderer/connection and a
  poll loop that ticks, renders to a dmabuf, and attaches it to the
  layer surface under the GPU render lock with a GL-fence wait
EOF
```

---

## Task 6: The throwaway validation overlay (standalone binary)

**Files:**

- Create: `system-overlays/validation-overlay/Cargo.toml`, `system-overlays/validation-overlay/src/main.rs`

- Modify: root `Cargo.toml` members

- [ ] **Step 1: Manifest**

```toml
[package]
name = "validation-overlay"
version = "0.1.0"
edition = "2024"
authors = ["Braiins Systems s.r.o."]
description = "Throwaway overlay validating the system-overlay framework"

[dependencies]
bmc-system-overlay.workspace = true
bmc-render.workspace = true
anyhow.workspace = true

[lints]
workspace = true
```

Add `bmc-system-overlay` to `[workspace.dependencies]` (path `system-overlays/bmc-system-overlay`) and the bin to
`members`.

- [ ] **Step 2: The overlay + main**

`src/main.rs`:

```rust
use std::time::Instant;

use bmc_render::renderer::Renderer;
use bmc_render::colors::Color;
use bmc_system_overlay::{run_standalone, LayerConfig, SystemOverlay, TickOutcome};

#[derive(Default)]
struct ValidationOverlay {
    // Content is static, so request exactly one render and then stay idle.
    rendered: bool,
}

impl SystemOverlay for ValidationOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig::fullscreen("bmc-validation")
    }

    fn tick(&mut self, _now: Instant) -> TickOutcome {
        // Render once; a later resize sets surface-dirty separately, so this
        // does not need to keep asking. No periodic wake.
        TickOutcome { wants_render: !self.rendered, next_wake: None }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        // Half-transparent green wash + an opaque marker box, to prove alpha
        // compositing over the live scene.
        let (w, h) = (size.0 as f32, size.1 as f32);
        r.fill_rect(0.0, 0.0, w, h, Color::from_rgba(0, 200, 0, 128));
        r.fill_rect(40.0, 40.0, 200.0, 120.0, Color::from_rgba(255, 255, 255, 255));
        r.draw_text("system overlay OK", 56.0, 96.0, 28.0, Color::from_rgba(0, 0, 0, 255));
        self.rendered = true;
    }
}

fn main() -> anyhow::Result<()> {
    run_standalone(Box::new(ValidationOverlay::default()))
}
```

`Color::from_rgba(u8,u8,u8,u8)` and `bmc_render::colors::Color` are confirmed correct. (Add a logging init only if the
crate already depends on a subscriber; otherwise rely on the compositor-side logs.)

- [ ] **Step 3: Build (host)**

Run: `nix develop -c cargo build -p validation-overlay` Expected: PASS.

- [ ] **Step 4: Commit**

```bash
nix fmt
git add system-overlays/validation-overlay Cargo.toml
git commit -F - <<'EOF'
system-overlays: Add throwaway validation overlay #BDK-416

- add a standalone binary using the framework that maps a fullscreen
  semi-transparent overlay with a marker box, used to validate the
  framework end to end against the compositor
EOF
```

---

## Task 7: Clippy

**Files:** none (verification)

Decision: **keep** the plan-1a `layer-shell-test-client`. It is a GPU-free `wl_shm` protocol smoke test;
`validation-overlay` exercises the full EGL/dmabuf/GPU path. Keeping both means that when the device misbehaves you can
isolate "protocol bug vs GPU bug" cheaply — valuable given the MMU-fault-prone GPU path (BDK-509). They serve different
purposes; neither supersedes the other.

- [ ] **Step 1: Clippy**

Run: `nix develop .#ci -c cargo clippy --workspace --tests -- -D warnings` Expected: PASS, no warnings. Fix any lint in
the new crates (`#[expect(..., reason = "…")]`, no ticket IDs in the reason).

- [ ] **Step 2: Format**

Run: `nix fmt` Expected: no changes; commit any formatting-only diff.

---

## Task 8: On-device verification

**Files:** none (verification)

- [ ] **Step 1: Build and deploy**

Build the ARM compositor (plan 1a) and the `validation-overlay` for ARM, and deploy with `scripts/nix-cargo-deploy.sh`
(it deploys native binaries). Set `DEVICE_IP`.

- [ ] **Step 2: Run the standalone overlay on device**

Run `validation-overlay` on the device against the running compositor (its own process, env `WAYLAND_DISPLAY`).
Expected:

- It connects, the compositor logs `new_layer_surface` + sends a configure, the overlay acks and maps.

- A half-transparent green wash with the white marker box appears **over** the cycling scene (scene shows through),
  confirming the framework renders through its own EGL/renderer, exports a DMA-BUF, and the compositor alpha-composites
  it.

- Touching the marker is delivered to the overlay (add a temporary log in `on_touch`), confirming input routing through
  the framework.

- [ ] **Step 3: Verify the standalone GPU lock is real**

`GpuRenderLock::from_env()` keys off the lock-path env var; a standalone overlay launched on device must inherit it or
the "lock" is a silent no-op and the spec's cross-context serialization is lost. Confirm the var the compositor/host use
(the one naming `/run/bmc-gpu-render.lock`) is set in `validation-overlay`'s environment when launched, and that it
contends with the compositor/host (e.g. log lock acquisition, or observe no MMU-fault under concurrent rendering). If it
is unset, the standalone launcher must export it.

- [ ] **Step 4: Verify clean teardown**

Kill `validation-overlay`. Expected: overlay disappears, scene repaints with no stale pixels, compositor logs
`layer_destroyed`, no buffer-leak warnings, RSS stable across repeated runs, no MMU-fault under an animating scene
(BDK-509 watch).

- [ ] **Step 5: Record results in the PR.**

---

## Self-review notes

- Spec coverage: implements the framework crate, the `SystemOverlay` trait, the tree-pipeline path (`TreeUi`) plus the
  immediate-mode escape hatch (the validation overlay draws via `Renderer` directly), the standalone entrypoint (own
  connection + renderer + loop), per-overlay input region (`InputRegion::None`/`Full`), the layer-shell exclusive-zone
  knob, and the full buffer-release round-trip (`OverlayRenderTarget` + the client's `wl_buffer.Release` handling) so
  the double buffer never stalls. The standalone render path takes the GPU render lock and GL-fence-waits before
  handoff, satisfying the spec's GPU-serialization constraint for standalone mode.
- Every commit compiles: `lib.rs` grows incrementally (each task appends its own `mod`/`pub use` in the commit that
  creates the file) — no comment-out/restore dance.
- Corrected API facts (not "confirm"): `Renderer` is `bmc_render::renderer::Renderer`; `Color` is
  `bmc_render::colors::Color`; `GpuRenderLock::lock(scope)` (no `acquire`);
  `InteractionState`=`bmc_render::interaction::…`, `NodeContext`=`bmc_render::tree::…`, `TaffyTree`=`taffy::prelude::…`;
  `DoubleBufferState::new(w,h,Depth)` takes no `EglContext`. Loop uses `bmc-widget`'s `poll_dispatch`
  (prepare_read→poll→read→dispatch), never a bare `poll(2)`+`dispatch_pending`.
- Reuse: the `bmc-widget` dmabuf/release helpers and `DoubleBufferState`/`SlotReleaseState` need `pub` widening from
  `pub(crate)` (Task 3 Step 1) — that is the intended reuse seam.
- Transparency: `scratch.begin_frame` clears opaque, so the framework re-clears the staging FBO to alpha 0 before
  drawing — without it a "transparent" overlay would be opaque.
- Deferred cleanup noted: the GL-fence wait is duplicated from the host (Task 4); hoisting it into `bmc-widget` so both
  share one copy is a follow-up.
- Out of scope: hosted mode + `bmc-wasm-host` integration (plan 1c); `deck_screen_edge_v1` and the swipe panel (spec
  Step 3/4).

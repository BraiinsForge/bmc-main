# System overlays 1a — compositor wlr-layer-shell foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the compositor accept a `wlr-layer-shell` client surface, composite it (with per-pixel alpha) above the
active scene, route touch to it honoring its input region and z-order, and track/import/release its buffer correctly —
including freeing it on unmap.

**Architecture:** Use Smithay's built-in `WlrLayerShellState` + `WlrLayerShellHandler` for the protocol only. This
compositor has a custom `SceneRenderer` (no Smithay `Space`/`layer_map`), so layer surfaces are tracked in a new
`CompositorState.layer_surfaces` registry, positioned by a pure anchor→geometry helper, imported into the existing
`texture_cache`, drawn after widgets in `render_scene`, and released on unmap. Buffer/eviction bookkeeping is a pure,
unit-tested helper. Layer changes mark full output damage (per-layer damage optimization is deferred).

**Tech Stack:** Rust, Smithay (pinned git rev `c114b88`), `wayland-protocols-wlr`, GLES via Smithay
`Frame::render_texture_from_to`, DRM/GBM/EGL. `bmc-openwrt` is cross-compiled for ARMv7.

**Context the executor needs:**

- Sub-plan 1a of BDK-416. Spec: `docs/superpowers/specs/2026-06-07-system-overlays-design.md`.
- `bmc-openwrt` builds only for ARM. Build/check with `nix develop .#ci -c cargo <cmd> -p bmc-openwrt`. The pure-logic
  tests in Task 1 live in `bmc-openwrt` and run in CI's nextest; locally try
  `nix develop .#ci -c cargo test -p bmc-openwrt <name>`, and if the cross target blocks host execution, note it and
  rely on CI for those tests (the GPU behavior is gated on device in Task 8 regardless).
- Always run cargo sandboxed. `nix fmt` (plain, no pipes) before each commit. No `--no-verify`. No ticket IDs in code
  comments. `#[expect]` not `#[allow]`. Never encode "no value" as `0`/`-1`; use `Option` or `expect("BUG: …")`.

---

## File Structure

- Create `bmc-openwrt/src/compositor/layer_surface.rs` — the `LayerEntry` registry type, the pure `layer_geometry` /
  `replace_buffer` / `paint_order` / `is_fullscreen_overlay` / `suppress_prepared` helpers, and their unit tests. Keeps
  layer logic out of the already-large `state.rs`.
- Modify `bmc-openwrt/src/compositor.rs` — add `mod layer_surface;`.
- Modify `bmc-openwrt/src/compositor/state.rs` — `layer_shell_state` + `layer_surfaces` registry; constructor wiring;
  `delegate_layer_shell!`; `WlrLayerShellHandler`; layer-surface commit routing; `layer_render_items`;
  `fullscreen_overlay_active`; extend `touch_focus_at`.
- Modify `bmc-openwrt/src/compositor/scene_renderer.rs` — import + composite layer buffers after widgets, drawn at the
  computed destination size.
- Modify `bmc-openwrt/src/compositor/egl_compositor.rs` — pass layer render items into `render_scene`; gate scene-drag
  on `!fullscreen_overlay_active()`; demote `Prepared`→`Dormant` in `emit_lifecycle_transitions` when a fullscreen
  overlay is up; re-emit lifecycle on overlay map/unmap.
- Create `system-overlays/layer-shell-test-client/` — a minimal standalone `wl_shm` layer-shell client used to verify
  the compositor on device (host + ARM build).

**Task order rationale:** the pure module (Task 1) compiles and tests green on its own. The global +
`delegate_layer_shell!` + the `WlrLayerShellHandler` are landed **together** in Task 2 (the macro requires the handler,
so splitting them would leave a knowingly-broken tree and break the Task 1 test gate). Everything after builds on a
compiling tree.

---

## Task 1: Pure layer helpers (geometry, buffer bookkeeping, ordering) — unit-tested

**Files:**

- Create: `bmc-openwrt/src/compositor/layer_surface.rs`

- Modify: `bmc-openwrt/src/compositor.rs`

- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the helpers and their tests (complete)**

Create `bmc-openwrt/src/compositor/layer_surface.rs`:

```rust
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::{Logical, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::{Anchor, Layer, LayerSurface, Margins};

/// Resolved client layer-surface state needed to place it.
pub struct LayerPlacement {
    pub size: Size<i32, Logical>,
    pub anchor: Anchor,
    pub margin: Margins,
}

/// Compute a layer surface's logical destination rectangle on an output of
/// `output` logical size. A zero size on an axis anchored to both opposite
/// edges stretches to fill that axis; otherwise the client's size is used.
#[must_use]
pub fn layer_geometry(p: &LayerPlacement, output: Size<i32, Logical>) -> Rectangle<i32, Logical> {
    let stretch_x = p.anchor.contains(Anchor::LEFT) && p.anchor.contains(Anchor::RIGHT);
    let stretch_y = p.anchor.contains(Anchor::TOP) && p.anchor.contains(Anchor::BOTTOM);

    let w = if p.size.w == 0 && stretch_x { output.w - p.margin.left - p.margin.right } else { p.size.w };
    let h = if p.size.h == 0 && stretch_y { output.h - p.margin.top - p.margin.bottom } else { p.size.h };

    let x = if stretch_x {
        p.margin.left
    } else if p.anchor.contains(Anchor::RIGHT) {
        output.w - w - p.margin.right
    } else if p.anchor.contains(Anchor::LEFT) {
        p.margin.left
    } else {
        (output.w - w) / 2
    };
    let y = if stretch_y {
        p.margin.top
    } else if p.anchor.contains(Anchor::BOTTOM) {
        output.h - h - p.margin.bottom
    } else if p.anchor.contains(Anchor::TOP) {
        p.margin.top
    } else {
        (output.h - h) / 2
    };

    Rectangle::from_loc_and_size((x, y), (w, h))
}

/// Stacking rank: higher draws on top.
#[must_use]
pub fn layer_rank(layer: Layer) -> u8 {
    match layer {
        Layer::Background => 0,
        Layer::Bottom => 1,
        Layer::Top => 2,
        Layer::Overlay => 3,
    }
}

/// Indices of `ranks` in paint order (bottom first). Stable within a rank, so
/// equal-rank surfaces keep registration order: later-registered paints last
/// (on top). Touch hit-testing iterates the reverse so the topmost wins.
#[must_use]
pub fn paint_order(ranks: &[u8]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..ranks.len()).collect();
    idx.sort_by_key(|&i| ranks[i]); // stable: preserves registration order within a rank
    idx
}

/// Swap a tracked buffer for a new one (or `None` to clear), returning the
/// previous buffer and its id so the caller can release the buffer and
/// invalidate the texture. Pure: the real types are filled in by the caller.
pub fn replace_buffer<B, I>(
    cur_buf: &mut Option<B>,
    cur_id: &mut Option<I>,
    new: Option<(B, I)>,
) -> (Option<B>, Option<I>) {
    let old_buf = cur_buf.take();
    let old_id = cur_id.take();
    if let Some((b, i)) = new {
        *cur_buf = Some(b);
        *cur_id = Some(i);
    }
    (old_buf, old_id)
}

/// One tracked layer-shell surface and its current buffer state.
pub struct LayerEntry {
    pub surface: LayerSurface,
    pub layer: Layer,
    /// Currently-committed buffer, or `None` when unmapped (NULL buffer).
    pub buffer: Option<WlBuffer>,
    /// ObjectId of the committed buffer, retained so an unmap (which carries
    /// no buffer object) can still evict the matching texture-cache entry.
    pub buffer_id: Option<ObjectId>,
    /// Last computed logical geometry, used to damage the vacated region on hide.
    pub last_geometry: Option<Rectangle<i32, Logical>>,
}

impl LayerEntry {
    pub fn new(surface: LayerSurface, layer: Layer) -> Self {
        Self { surface, layer, buffer: None, buffer_id: None, last_geometry: None }
    }
    pub fn is_mapped(&self) -> bool {
        self.buffer.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(size: (i32, i32), anchor: Anchor) -> LayerPlacement {
        LayerPlacement { size: size.into(), anchor, margin: Margins::default() }
    }

    #[test]
    fn fullscreen_stretches_all_edges() {
        let p = placement((0, 0), Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        assert_eq!(layer_geometry(&p, (1280, 480).into()), Rectangle::from_loc_and_size((0, 0), (1280, 480)));
    }

    #[test]
    fn bottom_right_corner_uses_client_size() {
        let p = placement((120, 40), Anchor::BOTTOM | Anchor::RIGHT);
        assert_eq!(layer_geometry(&p, (1280, 480).into()), Rectangle::from_loc_and_size((1160, 440), (120, 40)));
    }

    #[test]
    fn top_full_width_panel() {
        let p = placement((0, 200), Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
        assert_eq!(layer_geometry(&p, (1280, 480).into()), Rectangle::from_loc_and_size((0, 0), (1280, 200)));
    }

    #[test]
    fn unanchored_centers() {
        let p = placement((400, 100), Anchor::empty());
        assert_eq!(layer_geometry(&p, (1280, 480).into()), Rectangle::from_loc_and_size((440, 190), (400, 100)));
    }

    #[test]
    fn paint_order_is_stable_within_rank() {
        // ranks: Overlay(3), Top(2), Overlay(3) registered at indices 0,1,2.
        // Paint order: Top first, then the two Overlays in registration order.
        assert_eq!(paint_order(&[3, 2, 3]), vec![1, 0, 2]);
    }

    #[test]
    fn replace_buffer_new_returns_previous() {
        let mut buf = Some(10_u32);
        let mut id = Some(100_u32);
        let (old_buf, old_id) = replace_buffer(&mut buf, &mut id, Some((11, 101)));
        assert_eq!((old_buf, old_id), (Some(10), Some(100)));
        assert_eq!((buf, id), (Some(11), Some(101)));
    }

    #[test]
    fn replace_buffer_remove_clears_and_returns_previous() {
        let mut buf = Some(11_u32);
        let mut id = Some(101_u32);
        let (old_buf, old_id) = replace_buffer(&mut buf, &mut id, None);
        assert_eq!((old_buf, old_id), (Some(11), Some(101)));
        assert_eq!((buf, id), (None, None));
    }

    #[test]
    fn replace_buffer_remove_when_empty_is_noop() {
        let mut buf: Option<u32> = None;
        let mut id: Option<u32> = None;
        let (old_buf, old_id) = replace_buffer(&mut buf, &mut id, None);
        assert_eq!((old_buf, old_id), (None, None));
    }
}
```

- [ ] **Step 2: Register the module**

In `bmc-openwrt/src/compositor.rs`, add `mod layer_surface;` with the other module declarations.

- [ ] **Step 3: Run the tests**

Run: `nix develop .#ci -c cargo test -p bmc-openwrt layer_surface::tests` Expected: PASS (8 tests). The crate compiles
at this point (no `delegate_layer_shell!` yet). If the ARM cross target blocks host test execution, record that and
confirm the tests are green in CI instead.

- [ ] **Step 4: Commit**

```bash
nix fmt
git add bmc-openwrt/src/compositor/layer_surface.rs bmc-openwrt/src/compositor.rs
git commit -F - <<'EOF'
bmc: compositor: Add pure layer-surface helpers #BDK-416

- add LayerEntry registry type and pure helpers: anchor geometry,
  stacking rank, stable paint order, and buffer-slot replacement
- cover geometry, ordering, and buffer bookkeeping with unit tests
EOF
```

---

## Task 2: Layer-shell global, delegate, and handler (one compiling step)

**Files:**

- Modify: `bmc-openwrt/src/compositor/state.rs`

These land together: `delegate_layer_shell!` requires `WlrLayerShellHandler`, so adding the macro without the handler
would not compile.

- [ ] **Step 1: Imports**

At the top of `state.rs`, with the other `smithay::wayland::shell` imports, add:

```rust
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface, LayerSurfaceCachedState, WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::delegate_layer_shell;
```

Also extend the `smithay::utils` import at `state.rs:31` to include `Rectangle` (currently
`utils::{Logical, Point, Serial, Size}`):

```rust
    utils::{Logical, Point, Rectangle, Serial, Size},
```

- [ ] **Step 2: State fields**

In `struct CompositorState`, after `pub xdg_shell_state: XdgShellState,`, add:

```rust
    pub layer_shell_state: WlrLayerShellState,
    /// Tracked wlr-layer-shell surfaces, drawn above the scene.
    pub layer_surfaces: Vec<crate::compositor::layer_surface::LayerEntry>,
```

In `CompositorState::new`, near the `XdgShellState::new` call, add:

```rust
        let layer_shell_state = WlrLayerShellState::new::<Self>(&display_handle);
```

and initialize the two fields in the struct initializer: `layer_shell_state,`, `layer_surfaces: Vec::new(),`.

- [ ] **Step 3: Delegate**

In the delegate block (state.rs:981), after `delegate_xdg_shell!(self::CompositorState);`, add:

```rust
delegate_layer_shell!(self::CompositorState);
```

- [ ] **Step 4: Handler**

Add near the `XdgShellHandler` impl (state.rs:844):

```rust
impl WlrLayerShellHandler for CompositorState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
        layer: Layer,
        _namespace: String,
    ) {
        use crate::compositor::layer_surface::LayerEntry;
        self.layer_surfaces.push(LayerEntry::new(surface, layer));
        self.mark_full_output_damage();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        use crate::compositor::layer_surface::replace_buffer;
        if let Some(pos) = self
            .layer_surfaces
            .iter()
            .position(|e| e.surface.wl_surface() == surface.wl_surface())
        {
            let mut entry = self.layer_surfaces.remove(pos);
            let (old_buf, old_id) = replace_buffer(&mut entry.buffer, &mut entry.buffer_id, None);
            if let Some(buf) = old_buf {
                buf.release();
            }
            if let Some(id) = old_id {
                self.invalidated_buffers.push(id);
            }
            self.mark_full_output_damage();
        }
    }
}
```

- [ ] **Step 5: Build**

Run: `nix develop .#ci -c cargo build -p bmc-openwrt` Expected: PASS (global advertised, handler satisfied, tree
compiles).

- [ ] **Step 6: Commit**

```bash
nix fmt
git add bmc-openwrt/src/compositor/state.rs
git commit -F - <<'EOF'
bmc: compositor: Advertise and handle wlr-layer-shell #BDK-416

- create and delegate the layer-shell global
- implement WlrLayerShellHandler: track new surfaces, and on destroy
  release the buffer and queue its texture for eviction
EOF
```

---

## Task 3: Layer-surface commit handling

**Files:**

- Modify: `bmc-openwrt/src/compositor/state.rs`

- [ ] **Step 1: Route layer commits first**

In `CompositorHandler::commit` (state.rs:653), at the very start, add:

```rust
        if self.commit_layer_surface(surface) {
            return;
        }
```

- [ ] **Step 2: Implement `commit_layer_surface`**

Add in an `impl CompositorState` block. It resolves placement and layer from the cached state (so z-order and geometry
never go stale), sends the initial configure, processes the buffer via the pure `replace_buffer` helper, and drains any
queued frame callbacks unanswered (1a does not deliver them — overlays self-pace — but the fast path returns before the
normal handler, so it must clear them or they accumulate in surface state):

```rust
    /// Handle a commit for a tracked layer surface. Returns `true` if `surface`
    /// is a layer surface (and was handled), `false` otherwise.
    fn commit_layer_surface(&mut self, surface: &WlSurface) -> bool {
        use crate::compositor::layer_surface::{layer_geometry, replace_buffer, LayerPlacement};

        let Some(idx) = self
            .layer_surfaces
            .iter()
            .position(|e| e.surface.wl_surface() == surface)
        else {
            return false;
        };

        let layer_surface = self.layer_surfaces[idx].surface.clone();
        let needs_configure = layer_surface.has_pending_changes();

        // Read placement AND layer from the committed cached state, so a client
        // that changes its layer or anchor is reflected.
        let (placement, layer) = layer_surface.with_cached_state(|s: &LayerSurfaceCachedState| {
            (
                LayerPlacement { size: s.size, anchor: s.anchor, margin: s.margin },
                s.layer,
            )
        });
        self.layer_surfaces[idx].layer = layer;

        let output_w = i32::try_from(self.width).expect("BUG: logical display width fits i32");
        let output_h = i32::try_from(self.height).expect("BUG: logical display height fits i32");
        let geometry = layer_geometry(&placement, Size::from((output_w, output_h)));

        if needs_configure {
            layer_surface.with_pending_state(|state| {
                state.size = Some(geometry.size);
            });
            layer_surface.send_configure();
        }

        // Buffer handling. Collect bookkeeping into locals to avoid borrowing
        // self inside the with_states closure.
        let mut release: Option<WlBuffer> = None;
        let mut invalidate: Option<ObjectId> = None;
        let mut dirty: Option<ObjectId> = None;
        let mut had_buffer_change = false;

        with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attributes = guard.current();

            if let Some(assignment) = attributes.buffer.take() {
                had_buffer_change = true;
                let entry = &mut self.layer_surfaces[idx];
                match assignment {
                    BufferAssignment::NewBuffer(buffer) => {
                        let new_id = buffer.id();
                        let (old_buf, old_id) =
                            replace_buffer(&mut entry.buffer, &mut entry.buffer_id, Some((buffer, new_id.clone())));
                        release = old_buf;
                        invalidate = old_id;
                        dirty = Some(new_id);
                        entry.last_geometry = Some(geometry);
                    }
                    BufferAssignment::Removed => {
                        let (old_buf, old_id) =
                            replace_buffer(&mut entry.buffer, &mut entry.buffer_id, None);
                        release = old_buf;
                        invalidate = old_id;
                        // last_geometry stays so the renderer repaints the vacated region.
                    }
                }
            }
            attributes.damage.clear();
            // The fast path returns before the normal commit handler, so drain
            // any wl_surface.frame callbacks here. 1a does not deliver layer
            // frame callbacks (overlays self-pace in the framework); drop them
            // unanswered so they do not accumulate in surface state.
            let dropped = attributes.frame_callbacks.len();
            attributes.frame_callbacks.clear();
            if dropped > 0 {
                tracing::trace!("dropping {dropped} unanswered layer frame callback(s)");
            }
        });

        if let Some(buf) = release {
            buf.release();
        }
        if let Some(id) = invalidate {
            self.invalidated_buffers.push(id);
        }
        if let Some(id) = dirty {
            self.dirty_buffers.push(id);
        }
        if had_buffer_change {
            self.mark_full_output_damage();
        }

        true
    }
```

Note: `self.layer_surfaces[idx]` is mutated inside the `with_states` closure while only locals are touched outside it —
`release`/`invalidate`/`dirty` are applied to `self` after the closure returns, so there is no aliasing of
`self.dirty_buffers`/`self.invalidated_buffers` during the borrow. If the borrow checker still objects to
`&mut self.layer_surfaces[idx]` inside the closure capturing `self`, split the closure: read `attributes.buffer.take()`
into a local first, then mutate `self.layer_surfaces[idx]` after `with_states` returns.

- [ ] **Step 3: Build**

Run: `nix develop .#ci -c cargo build -p bmc-openwrt` Expected: PASS.

- [ ] **Step 4: Commit**

```bash
nix fmt
git add bmc-openwrt/src/compositor/state.rs
git commit -F - <<'EOF'
bmc: compositor: Handle layer-surface commits #BDK-416

- send the initial configure from resolved anchor geometry, tracking
  the client's current layer for z-order
- record the committed buffer via the pure buffer-slot helper, release
  the previous one on replace and unmap, and damage on change
- drain queued frame callbacks unanswered so they do not accumulate
EOF
```

---

## Task 4: Composite layer surfaces above the scene

**Files:**

- Modify: `bmc-openwrt/src/compositor/scene_renderer.rs`

- Modify: `bmc-openwrt/src/compositor/state.rs`

- Modify: `bmc-openwrt/src/compositor/egl_compositor.rs`

- [ ] **Step 1: `layer_render_items` (paint order, with sizes)**

In `state.rs`, add:

```rust
    /// Mapped layer surfaces as (buffer, logical destination rect) in paint
    /// order (bottom first). Destination carries both location and size.
    #[must_use]
    pub fn layer_render_items(&self) -> Vec<(WlBuffer, Rectangle<i32, Logical>)> {
        use crate::compositor::layer_surface::{layer_rank, paint_order};
        let mapped: Vec<&crate::compositor::layer_surface::LayerEntry> =
            self.layer_surfaces.iter().filter(|e| e.is_mapped()).collect();
        let ranks: Vec<u8> = mapped.iter().map(|e| layer_rank(e.layer)).collect();
        paint_order(&ranks)
            .into_iter()
            .filter_map(|i| {
                let e = mapped[i];
                Some((e.buffer.clone()?, e.last_geometry?))
            })
            .collect()
    }
```

- [ ] **Step 2: Add `Logical` to the renderer imports**

In `scene_renderer.rs:16`, extend the `smithay::utils` import to include `Logical` (currently
`utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform}`):

```rust
    utils::{Buffer as BufferCoord, Logical, Physical, Rectangle, Size, Transform},
```

- [ ] **Step 3: Extend `render_scene` to import + draw layers at their destination size**

Change the `render_scene` signature (scene_renderer.rs:244) to add a `layers` slice after `buffers`:

```rust
        layers: &[(WlBuffer, Rectangle<i32, Logical>)],
```

After `self.import_textures(buffers, dirty);`, import layer buffers (they share `texture_cache` + `dirty`):

```rust
        for (buffer, _) in layers {
            let buffer_id = buffer.id();
            if let Ok(dmabuf) = get_dmabuf(buffer) {
                if dirty.contains(&buffer_id) {
                    if let Ok(texture) = self.egl.renderer().import_dmabuf(dmabuf, None) {
                        self.texture_cache.insert(buffer_id, texture);
                    }
                }
            } else if let Ok(texture) = self.egl.renderer().import_shm_buffer(buffer, None, &[]) {
                self.texture_cache.insert(buffer_id, texture);
            }
        }
```

After the widget render loop (after the `stopwatch_stop!(self.compose_w)` at ~line 373), before `frame.finish()`, draw
the layers in paint order using the computed destination **size** (`geo.size`), not the texture size:

```rust
        #[expect(clippy::cast_possible_wrap, reason = "output dims are within i32")]
        let (output_w, output_h) = (self.output.width() as i32, self.output.height() as i32);
        for (buffer, geo) in layers {
            let Some(texture) = self.texture_cache.get(&buffer.id()) else {
                continue;
            };
            let tex_size = texture.size();
            // Destination uses the layer's computed geometry size, so render,
            // hit-test, and damage all agree on the rectangle.
            let dst = place_widget(
                geo.loc.x,
                geo.loc.y,
                geo.size.w,
                geo.size.h,
                output_w,
                output_h,
                self.scanout_transform,
            );
            let src: Rectangle<f64, BufferCoord> = Rectangle::from_loc_and_size(
                (0.0, 0.0),
                (f64::from(tex_size.w), f64::from(tex_size.h)),
            );
            let damage = texture_damage_rect(dst);
            if let Err(e) = frame.render_texture_from_to(
                texture,
                src,
                dst,
                &[damage],
                &[],
                scanout_transform(self.scanout_transform),
                1.0,
                None,
                &[],
            ) {
                tracing::warn!("Failed to render layer surface: {:?}", e);
            }
        }
```

Per-pixel transparency comes from an `ARGB8888` layer buffer; Smithay's GLES frame blends textures carrying alpha, and
the `1.0` argument is the global opacity multiplier (kept full). The layer import + draw happen inside the existing
`gpu_render_lock` scope already held across `render_scene` (acquired at scene_renderer.rs:261), so cross-context GPU
serialization is preserved.

- [ ] **Step 4: Pass layer items at the call site**

In `egl_compositor.rs`, collect items before the `render_scene(...)` call (around line 629) and add them to the argument
list in the new `layers` position:

```rust
            let layer_items = app_state.compositor.layer_render_items();
```

Add `&layer_items,` to the `render_scene(...)` args (after the widget `buffers` arg). **Do not** add an
`invalidate_textures` block here — `egl_compositor.rs:633-636` already drains `invalidated_buffers` and calls
`renderer.invalidate_textures(...)` every iteration, and Task 3 pushes layer evictions into that same
`invalidated_buffers` vec, so layer-buffer eviction is already handled.

- [ ] **Step 5: Build**

Run: `nix develop .#ci -c cargo build -p bmc-openwrt` Expected: PASS.

- [ ] **Step 6: Commit**

```bash
nix fmt
git add bmc-openwrt/src/compositor/scene_renderer.rs bmc-openwrt/src/compositor/state.rs bmc-openwrt/src/compositor/egl_compositor.rs
git commit -F - <<'EOF'
bmc: compositor: Composite layer surfaces above the scene #BDK-416

- import layer buffers and draw them after widgets in paint order at
  their computed destination size, with per-pixel alpha over the scene
EOF
```

---

## Task 5: Touch routing, scene-drag suppression, and neighbor demotion for fullscreen overlays

**Files:**

- Modify: `bmc-openwrt/src/compositor/state.rs`

The compositor performs focus selection, so it must honor each layer surface's Wayland input region (an empty region =
passive, falls through) and must hand touch to the **topmost-painted** surface (matching Task 4's paint order).

- [ ] **Step 1: Extend `touch_focus_at`**

Replace `touch_focus_at` (state.rs:397) so it tests layers before widgets, in reverse paint order (topmost first), and
only accepts a layer whose input region contains the surface-local point:

```rust
    #[must_use]
    pub fn touch_focus_at(&self, x: f64, y: f64) -> Option<(WlSurface, Point<f64, Logical>)> {
        use crate::compositor::layer_surface::{layer_rank, paint_order};

        // Layer pass: topmost painted surface that is mapped, contains the
        // point, and accepts input at that point.
        let mapped: Vec<&crate::compositor::layer_surface::LayerEntry> =
            self.layer_surfaces.iter().filter(|e| e.is_mapped()).collect();
        let ranks: Vec<u8> = mapped.iter().map(|e| layer_rank(e.layer)).collect();
        for &i in paint_order(&ranks).iter().rev() {
            let entry = mapped[i];
            let Some(g) = entry.last_geometry else { continue };
            let (gx, gy) = (f64::from(g.loc.x), f64::from(g.loc.y));
            let (gw, gh) = (f64::from(g.size.w), f64::from(g.size.h));
            if !(x >= gx && x < gx + gw && y >= gy && y < gy + gh) {
                continue;
            }
            let surface = entry.surface.wl_surface();
            if !surface.is_alive() {
                continue;
            }
            // Honor the surface input region: None means whole surface accepts
            // input; an explicit region must contain the surface-local point.
            let local = Point::<f64, Logical>::from((x - gx, y - gy));
            let accepts = with_states(surface, |states| {
                let mut guard = states.cached_state.get::<SurfaceAttributes>();
                match &guard.current().input_region {
                    None => true,
                    Some(region) => region.contains(local.to_i32_round()),
                }
            });
            if accepts {
                return Some((surface.clone(), Point::from((gx, gy))));
            }
            // Region rejects this point: fall through to surfaces/widgets below.
        }

        // Widget pass (unchanged).
        let scene = self.widgets.active_scene();
        for widget in &scene.widgets {
            if !widget.visible {
                continue;
            }
            let (wx, wy) = (f64::from(widget.position.x), f64::from(widget.position.y));
            let (ww, wh) = (f64::from(widget.size.width), f64::from(widget.size.height));
            if x >= wx
                && x < wx + ww
                && y >= wy
                && y < wy + wh
                && let Some(surface) = self.render_surfaces.get(&widget.instance_id)
                && surface.is_alive()
            {
                return Some((surface.clone(), Point::from((wx, wy))));
            }
        }
        None
    }
```

The input-region API is `SurfaceAttributes.input_region: Option<RegionAttributes>`, with
`RegionAttributes::contains<P: Into<Point<i32, Logical>>>(&self, point: P) -> bool`. `None` ⇒ the whole surface accepts
input; an explicit (possibly empty) region ⇒ membership test. `region.contains(local.to_i32_round())` is the call.

- [ ] **Step 2: Pure helpers for the fullscreen-overlay predicate and neighbor demotion (unit-tested)**

A fullscreen overlay must disable scene-drag and stop warming scene neighbors. Because `Prepared` literally means
"immediate swipe-neighbor" (`widget_tracker.rs:377`) and `Prepared` holds one render target while `Dormant` holds zero,
demoting neighbors `Prepared`→`Dormant` both frees a buffer and is the natural "no swipe target" state. Add these pure
helpers to `layer_surface.rs` (testable without Wayland):

```rust
/// True if a mapped overlay-layer surface at `geo` covers the whole output.
#[must_use]
pub fn is_fullscreen_overlay(layer: Layer, geo: Rectangle<i32, Logical>, output: Size<i32, Logical>) -> bool {
    layer == Layer::Overlay
        && geo.loc.x <= 0
        && geo.loc.y <= 0
        && geo.size.w >= output.w
        && geo.size.h >= output.h
}

/// Demote every `Prepared` entry to `Dormant`. Used while a fullscreen overlay
/// is up: there is nothing to swipe to, so no neighbor should stay warm.
pub fn suppress_prepared(states: &mut std::collections::HashMap<InstanceId, LifecycleState>) {
    for state in states.values_mut() {
        if *state == LifecycleState::Prepared {
            *state = LifecycleState::Dormant;
        }
    }
}
```

(Import `LifecycleState` and `InstanceId` into `layer_surface.rs`.) Add unit tests: `is_fullscreen_overlay` true for an
Overlay surface at `(0,0)` sized ≥ output, false for a smaller corner surface, false for a `Top`-layer surface;
`suppress_prepared` turns `Prepared`→`Dormant` and leaves `Visible`/`Entering`/`Leaving` untouched.

Then add the compositor method (in `state.rs`):

```rust
    #[must_use]
    pub fn fullscreen_overlay_active(&self) -> bool {
        use crate::compositor::layer_surface::is_fullscreen_overlay;
        let output = Size::from((
            i32::try_from(self.width).expect("BUG: logical display width fits i32"),
            i32::try_from(self.height).expect("BUG: logical display height fits i32"),
        ));
        self.layer_surfaces.iter().any(|e| {
            e.is_mapped()
                && e.last_geometry
                    .is_some_and(|g| is_fullscreen_overlay(e.layer, g, output))
        })
    }
```

- [ ] **Step 3: Wire drag suppression, neighbor demotion, and re-emit on overlay map/unmap**

1. **Disable scene-drag.** At `egl_compositor.rs:1236`, extend the arbitration guard:

```rust
        if drag_activated
            && !self.scene_drag_active
            && self.compositor.widgets.can_drag()
            && !self.compositor.fullscreen_overlay_active()
        {
```

2. **Demote neighbors when emitting lifecycle.** In `emit_lifecycle_transitions` (`egl_compositor.rs:1436`), right after
   `let next = state.compositor.widgets.lifecycle_states();`, post-process before `lifecycle.step(&next)`:

```rust
    let mut next = state.compositor.widgets.lifecycle_states();
    if state.compositor.fullscreen_overlay_active() {
        crate::compositor::layer_surface::suppress_prepared(&mut next);
    }
```

(Apply the same two lines at any other production `lifecycle_states()` emission path that feeds `lifecycle.step` —
verify the call sites at `egl_compositor.rs:915` and `:1786`; leave the test-only call sites alone.)

3. **Re-emit when the predicate flips.** A layer map/unmap happens during Wayland dispatch, not on a scene command, so
   the lifecycle won't re-emit on its own. In the compositor event loop, after client requests are dispatched (where
   layer commits are processed), compare `self.compositor.fullscreen_overlay_active()` to a stored
   `last_fullscreen_overlay_active: bool` on `AppState`; when it changes, call `emit_lifecycle_transitions(self)` and
   store the new value. This pushes neighbors to `Dormant` when a fullscreen overlay maps and restores them to
   `Prepared` when it unmaps.

- [ ] **Step 4: Build**

Run: `nix develop .#ci -c cargo build -p bmc-openwrt` Expected: PASS. Run the new helper tests:
`nix develop .#ci -c cargo test -p bmc-openwrt layer_surface::tests` (now also covers
`is_fullscreen_overlay`/`suppress_prepared`).

- [ ] **Step 5: Commit**

```bash
nix fmt
git add bmc-openwrt/src/compositor/state.rs bmc-openwrt/src/compositor/layer_surface.rs bmc-openwrt/src/compositor/egl_compositor.rs
git commit -F - <<'EOF'
bmc: compositor: Suppress scene-drag under fullscreen overlays #BDK-416

- hit-test mapped layer surfaces before scene widgets, topmost painted
  first, honoring the Wayland input region so passive overlays fall
  through
- while a fullscreen overlay is mapped, disable scene-drag and demote
  scene-swipe neighbors from Prepared to Dormant, freeing their buffers
- re-emit lifecycle when an overlay maps or unmaps
EOF
```

---

## Task 6: Minimal standalone layer-shell test client (host + ARM)

**Files:**

- Create: `system-overlays/layer-shell-test-client/Cargo.toml`, `system-overlays/layer-shell-test-client/src/main.rs`
- Modify: root `Cargo.toml` members

Verification fixture: connects, creates a fullscreen `Overlay`-layer surface with a semi-transparent `ARGB8888` `wl_shm`
buffer, commits, and on `Configure` acks + paints. Proves the global, configure, alpha compositing, touch routing, and
free-on-exit.

- [ ] **Step 1: Manifest**

```toml
[package]
name = "layer-shell-test-client"
version = "0.1.0"
edition = "2024"
authors = ["Braiins Systems s.r.o."]
description = "Minimal wlr-layer-shell client for verifying the compositor"

[dependencies]
wayland-client.workspace = true
wayland-protocols-wlr = { workspace = true, features = ["client"] }
wayland-protocols = { workspace = true, features = ["client"] }

[lints]
workspace = true
```

Add `wayland-protocols-wlr` to `[workspace.dependencies]` pinned to `0.3.12` (matching the lockfile) if absent; add the
crate to `members`.

- [ ] **Step 2: Write the client**

`system-overlays/layer-shell-test-client/src/main.rs` — a single-roundtrip client. Bind `wl_compositor`, `wl_shm`,
`zwlr_layer_shell_v1`, `wl_seat`; create a surface; `get_layer_surface` on `Overlay` anchored to all four edges, size
`(0,0)`; commit; on the first `zwlr_layer_surface_v1::Event::Configure { serial, width, height }` →
`ack_configure(serial)`, draw a premultiplied 50%-alpha green fill into a `wl_shm` `Argb8888` buffer of that size,
attach, commit; the pixel value is `0x80008000` in `0xAARRGGBB` notation (A=0x80, premultiplied G=0x80), written as a
native-endian `u32` so the in-memory bytes are B,G,R,A on little-endian; on `Closed` → exit. The content is static, so
it renders once on configure and does not request `wl_surface.frame`. Use an `memfd`/tmpfile-backed `wl_shm_pool`.
Protocol objects:

```rust
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
```

Implement the manual `Dispatch` impls (registry bind; `Configure`/`Closed`; no-op for the rest). Touch support is
**required** for the Task 8 verification: bind `wl_seat`, acquire `wl_touch` on the capabilities event, and `eprintln!`
the coordinates on `wl_touch::Event::Down`. Keep it under ~250 lines.

- [ ] **Step 3: Build for host (sanity)**

Run: `nix develop -c cargo build -p layer-shell-test-client` Expected: PASS on host.

- [ ] **Step 4: Build for ARM**

Run: `nix develop .#armv7-glibc-release -c cargo build -p layer-shell-test-client --release` Expected: PASS. The binary
is at `target/armv7-unknown-linux-gnueabihf/release/layer-shell-test-client`. (Confirm the target triple/profile names
against `workspace.nix`; use the same shell the compositor cross-build uses.)

- [ ] **Step 5: Commit**

```bash
nix fmt
git add system-overlays/layer-shell-test-client Cargo.toml
git commit -F - <<'EOF'
system-overlays: Add wlr-layer-shell test client #BDK-416

- add a minimal standalone layer-shell client that maps a fullscreen
  semi-transparent overlay, used to verify the compositor
EOF
```

---

## Task 7: Workspace clippy

**Files:** none (verification)

- [ ] **Step 1: Clippy**

Run: `nix develop .#ci -c cargo clippy --workspace --tests -- -D warnings` Expected: PASS, no warnings. Fix lints in
touched files (`#[expect(..., reason = "…")]`, no ticket IDs in the reason).

- [ ] **Step 2: Format**

Run: `nix fmt` Expected: no changes; commit any formatting-only diff.

---

## Task 8: On-device verification

**Files:** none (verification)

Acceptance gate for the GPU/compositing behavior, which is not unit-testable.

- [ ] **Step 1: Build and deploy the compositor**

Build the ARM compositor and deploy with `scripts/nix-cargo-deploy.sh` (deploys compositor + native binaries; see
`docs/nix-device-scripts.md`). Set `DEVICE_IP` (this is a Braiins Deck).

- [ ] **Step 2: Copy and run the test client on the device**

Copy the ARM binary built in Task 6 Step 4 to the device and run it against the live compositor:

```bash
scp target/armv7-unknown-linux-gnueabihf/release/layer-shell-test-client root@"$DEVICE_IP":/tmp/
ssh root@"$DEVICE_IP" 'WAYLAND_DISPLAY=$(ls /run/user/*/wayland-* 2>/dev/null | head -1 | xargs basename) /tmp/layer-shell-test-client'
```

(Confirm the device's Wayland socket path/env; the widgets use the same socket. Adjust the `WAYLAND_DISPLAY` discovery
to match the device.) Expected:

- Compositor logs `new_layer_surface` and sends a configure; the client acks and maps.

- A semi-transparent green fills the screen **over** the cycling scene (scene shows through) — alpha compositing above
  widgets.

- Touching the screen routes to the client (touch-down log) — confirms layer touch priority.

- **Dragging horizontally over the fullscreen overlay does NOT cycle scenes** — scene-drag is suppressed while the
  overlay is up.

- When the overlay maps, scene-swipe neighbors transition to `Dormant` (watch the lifecycle events / host logs); each
  demoted neighbor frees its render-target buffer. They return to `Prepared` after the overlay unmaps.

- [ ] **Step 3: Verify free-on-hide / exit and no fault**

Kill the client. Expected:

- Overlay disappears, scene repaints with **no stale green pixels** (damage-on-unmap + texture eviction).

- `layer_destroyed` logged; no buffer-leak warnings; RSS stable across repeated run/exit cycles (spot-check `free`/RSS).

- No MMU-fault / scene-freeze with an animating scene for a minute (BDK-509 watch). Layer compositing rode the existing
  `gpu_render_lock` (Task 4 Step 3), so cross-context serialization held.

- [ ] **Step 4: Record results (screenshots/log excerpts) in the PR.** If any point fails, stop and debug before plan
  1b.

---

## Self-review notes

- Spec coverage: layer-shell global + handler, alpha compositing above the scene at the computed destination size, the
  buffer registry with release + eviction (pure, unit-tested in Task 1), damage-on-hide (full damage), touch priority
  honoring input regions and z-order, and — for a fullscreen overlay — scene-drag suppression plus neighbor demotion
  `Prepared`→`Dormant` (which frees one buffer per neighbor, since `Prepared` holds a render target and `Dormant` does
  not). The partial-panel reveal trigger and the screen-edge gesture remain spec Step 3.
- Automated tests now cover the silent-regression risks the spec named: the buffer-slot transitions (`replace_buffer`:
  new/replace/remove/empty) and the stacking/paint order (`paint_order`) are pure functions with unit tests in Task 1.
  Only the GPU compositing itself is device-verified (Task 8) — that part genuinely has no host-runnable unit test
  (ARM-cross + GLES/DRM).
- Ordering consistency: render (`layer_render_items`) and touch (`touch_focus_at`) both use `paint_order`; touch
  iterates its reverse, so the topmost-painted surface receives touch. Within a layer, later-registered is on top, for
  both.
- Frame callbacks: deliberately **not** handled in 1a. The test client renders once on `Configure` and never blocks on
  `frame` `done`. Overlay redraw pacing is the framework's own `tick`/`next_wake` scheduling (plan 1b), not compositor
  frame callbacks. If frame-callback-driven pacing is later wanted for an animating layer client, route it through the
  existing paced widget path (`FRAME_CALLBACK_MIN_INTERVAL` + presented-gating in
  `send_frame_callbacks_for_presented_widgets`) rather than adding a second, unpaced path.
- GPU fence (spec's first-class constraint), stated precisely: the 1a test client uses `wl_shm`, which carries no
  per-buffer dmabuf fence; layer compositing rides the existing `gpu_render_lock` held across `render_scene`, preserving
  cross-context serialization. The dmabuf fence-before-sample handoff is only exercised once hosted overlays export
  dmabufs in plan 1c.
- Known risk to confirm against the compiler: the `with_states` borrow in `commit_layer_surface` (mutating
  `self.layer_surfaces[idx]` inside the closure) — the split-the-closure fallback is documented at that step. The
  Smithay input-region accessor (`SurfaceAttributes.input_region` / `RegionAttributes::contains`) signature is flagged
  at Task 5 Step 1.
- Out of scope: `revealed`/`hidden` events, `deck_screen_edge_v1`, neighbor→Dormant (spec Step 3); hosted-mode dmabuf
  fence handoff (plan 1c).

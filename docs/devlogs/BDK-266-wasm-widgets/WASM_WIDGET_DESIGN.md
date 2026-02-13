# Design Document: WASM Widget for Corinthia Hotel Demo

**Ticket:** BDK-266 / BDK-226 Integration
**Author:** Claude (AI Assistant)
**Date:** 2026-02-11
**Revision:** 4 (Implementation complete)

---

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Crate location** | Cross-repo via symlink | Keep `bmc-wasm-runtime` in BDK-266, symlink into Corinthia workspace |
| **Widget role** | Scene widget | Part of scene cycling, no touch events needed |
| **WASM distribution** | Pre-built files | Deploy `.wasm` to `/mnt/data/wasm/`, reference by path |
| **Testing approach** | Test mode + hardware | Test mode bypasses WASM; full testing on physical Deck |
| **Touch events** | Ignored (Phase 1) | Scene widgets don't receive touch - compositor intercepts for gestures |
| **Cross-compilation** | ✓ Verified compatible | femtovg/glow proven in Corinthia; wasmi/cosmic-text/taffy are pure Rust |

---

## 0. Prerequisites Verification

### Cross-Compilation for ARMv7

The `bmc-wasm-runtime` crate must cross-compile for `armv7-unknown-linux-musleabihf`. Verification status:

| Dependency | Version | Status | Evidence |
|------------|---------|--------|----------|
| **femtovg** | 0.9 | ✓ Verified | Used in `widgets/settings/` (Corinthia), deployed to target |
| **glow** | 0.16 | ✓ Verified | Same as femtovg, works with smithay EGL backend |
| **wasmi** | 0.45 | ✓ Pure Rust | WASM interpreter, no native C dependencies |
| **cosmic-text** | 0.17 | ✓ Pure Rust | Text shaping with swash feature (pure Rust) |
| **taffy** | 0.9 | ✓ Pure Rust | Flexbox layout engine, no native deps |

**Conclusion:** All dependencies are either already proven in the Corinthia workspace (femtovg, glow via smithay) or are pure Rust crates that cross-compile without issues.

---

## 1. Executive Summary

This document describes the design for a new widget type in the Corinthia hotel demo (BDK-226) that runs WebAssembly applications using the `bmc-wasm-runtime` (BDK-266). The widget will allow custom WASM-based UI overlays to run on the physical Braiins Deck hardware alongside existing widgets.

---

## 2. Background

### 2.1 Corinthia Demo (BDK-226) - Last 15 Commits Analysis

The Corinthia demo is a GPU-accelerated Wayland compositor for hotel room control panels. Recent development focused on:

| Date | Focus Area | Key Changes |
|------|------------|-------------|
| Feb 5 | **Performance** | Fixed settings widget render loop, reduced GPU overdraw, skip drawing when overlay covers screen |
| Feb 5 | **Touch latency** | Register touch fd with calloop for immediate wake, per-widget damage regions |
| Feb 5 | **Render optimization** | `glFlush()` vs `glFinish()` (3-8ms savings), frame skipping for static scenes |
| Feb 5 | **Gestures** | Drag-to-follow continuous gesture model, velocity-based scene transitions |
| Feb 3 | **Infrastructure** | Cross-compilation fixes for ARMv7 glibc, custom SSH port support |
| Jan 28 | **Deployment** | `deploy_corinthia.py` script, splash screen, texture caching fix |
| Jan 27-28 | **Settings UI** | FemtoVG-based settings overlay with 8-page navigation, brightness/volume sliders |
| Jan 27 | **Hardware wiring** | `set_setting` protocol for brightness, volume, restart, wifi_reset |

**Current Widget Types:**
1. **Picture Widget** - Static image display (simplest)
2. **EGL Demo Widget** - Raw OpenGL triangle (GPU showcase)
3. **Settings Overlay** - FemtoVG-based rich UI (most complex, reference implementation)

### 2.2 Settings Overlay Widget Architecture (Reference)

The settings overlay (`widgets/settings/`) is the most sophisticated widget and serves as the reference implementation. Key architectural patterns:

#### Two-FBO Y-Flip Pipeline

FemtoVG assumes a window system that flips Y on buffer swap. When rendering to FBO directly, the output is Y-inverted. The settings widget solves this with a two-FBO pipeline:

```
FemtoVG Canvas
  │ renders to Staging FBO (regular GL texture + stencil)
  │ content is Y-inverted in FBO coords
  │
  ▼
Staging FBO (RGBA texture + stencil RBO)
  │ blit_to_export() with Y-flip shader
  │ fullscreen quad, V coords flipped (1→0)
  │
  ▼
Export FBO (EGLImage-backed GBM BO)
  │ correct orientation
  │
  ▼
DMA-BUF → Wayland → Compositor
```

**Critical:** The WASM runtime uses FemtoVG, so the same two-FBO pipeline is required.

#### Touch Event Handling

The settings widget implements:
- **Velocity prediction** with 60ms lookahead for responsive sliders
- **Swipe detection** with 80px threshold
- **Hit-testing** per UI page (8-page state machine)
- **Slider dragging** with continuous updates

#### Wayland Integration

- Uses `wayland-client` crate with `Dispatch` trait implementations
- Binds: `wl_compositor`, `deck_widget_manager_v1`, `zwp_linux_dmabuf_v1`
- Creates `deck_widget_surface_v1` role via `get_widget_surface()`
- Handles events: `setting`, `shutdown`, `touch_down/motion/up`

### 2.3 WASM Runtime (BDK-266) - Architecture

The `bmc-wasm-runtime` provides:

- **WASM Execution**: wasmi interpreter with fuel metering (10M instructions/frame)
- **GPU Rendering**: FemtoVG + OpenGL ES for hardware-accelerated graphics
- **Layout Engine**: Taffy for host-side flexbox layout
- **Text Shaping**: cosmic-text for proper font rendering
- **Animation System**: Host-computed animations and transitions
- **SDK Versioning**: Runtime compatibility checking

**Key API:**
```rust
pub struct WasmWidgetRuntime {
    // Creates runtime from WASM bytes + GL context + target FBO
    pub unsafe fn new<F>(
        wasm_bytes: &[u8],
        load_fn: F,
        width: u32,
        height: u32,
        fbo_id: u32,  // Staging FBO for FemtoVG's set_screen_target()
    ) -> Result<Self>;

    // Render a frame (calls WASM render function)
    pub fn render(&mut self, delta_ms: u32) -> Result<()>;

    // Access GPU renderer for begin_frame/flush
    pub fn renderer(&mut self) -> &mut FemtoVgRenderer;

    // Check if WASM wants another frame
    pub fn wants_next_frame(&self) -> bool;

    // Get SDK version tuple
    pub fn sdk_version(&self) -> (u32, u32, u32);

    // Push touch events (future use)
    pub fn push_touch_event(&mut self, event: TouchEvent);
}
```

---

## 3. Requirements

### 3.1 Functional Requirements

| ID | Requirement |
|----|-------------|
| FR-1 | Widget loads and executes `.wasm` files specified in scene configuration |
| FR-2 | Widget renders WASM output to Wayland surface via DMA-BUF |
| FR-3 | Widget forwards touch events from compositor to WASM runtime |
| FR-4 | Widget respects settings changes (brightness, night mode, etc.) |
| FR-5 | Widget participates in scene cycling and overlay system |
| FR-6 | WASM binaries must be < 40KB (optimized) for reasonable load times |

### 3.2 Non-Functional Requirements

| ID | Requirement |
|----|-------------|
| NFR-1 | 60 FPS rendering on STM32MP157 + Vivante GC400 GPU |
| NFR-2 | Touch latency < 32ms (2 frames) |
| NFR-3 | Memory usage < 8MB per WASM widget instance |
| NFR-4 | Graceful degradation on WASM fuel exhaustion |

---

## 4. Architecture

### 4.1 High-Level Design

```
┌──────────────────────────────────────────────────────────────────┐
│                    Corinthia Compositor                           │
│  (bmc-openwrt/compositor/)                                       │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │ Widget Coordinator → spawns widget processes                 │ │
│  └──────────────────────────┬──────────────────────────────────┘ │
│                             │                                     │
└─────────────────────────────┼─────────────────────────────────────┘
                              │ spawn
                              ▼
┌──────────────────────────────────────────────────────────────────┐
│                    bmc-widget-wasm (NEW)                          │
│                                                                   │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────────────────┐ │
│  │ Wayland      │  │ EGL/GBM      │  │ bmc-wasm-runtime        │ │
│  │ Client       │  │ Two-FBO      │  │                         │ │
│  │              │  │ Pipeline     │  │ ┌─────────────────────┐ │ │
│  │ - wl_surface │  │              │  │ │ WasmWidgetRuntime   │ │ │
│  │ - DMA-BUF    │  │ - Staging    │  │ │ - wasmi engine      │ │ │
│  │ - deck_widget│  │   FBO        │  │ │ - FemtoVgRenderer   │ │ │
│  │   _v1 proto  │  │ - Export FBO │  │ │ - Touch handling    │ │ │
│  └──────┬───────┘  │ - Y-flip     │  │ └─────────────────────┘ │ │
│         │          │   blit       │  └───────────┬─────────────┘ │
│         │          └──────┬───────┘              │               │
│         │    buffer       │     render           │               │
│         │◄────────────────┤◄─────────────────────┘               │
│         │                 │                                      │
│         ▼                 │                                      │
│  Wayland protocol         │                                      │
│  (touch, settings)        └─── GPU commands ──► GPU              │
└──────────────────────────────────────────────────────────────────┘
```

### 4.2 Component Breakdown

#### 4.2.1 New Crate: `widgets/wasm/`

```
widgets/wasm/
├── Cargo.toml
├── manifest.json
└── src/
    ├── main.rs          # Entry point, logging setup, test mode detection
    ├── wayland.rs       # Wayland client, WASM/test mode dispatch
    ├── egl.rs           # Two-FBO pipeline with EGLImage cleanup
    └── test_renderer.rs # Test mode rendering (bypasses WASM)
```

#### 4.2.2 Dependencies

```toml
[dependencies]
# WASM runtime (from BDK-266)
bmc-wasm-runtime = { path = "../../../jku/BDK-266-wasm/bmc-wasm-runtime" }

# Widget protocol (from Corinthia)
bmc-widget-protocol = { path = "../../bmc-widget-protocol" }

# Wayland (same as settings widget)
wayland-client = "0.31"
wayland-protocols = { version = "0.32", features = ["client"] }

# GPU buffer management (same as settings widget)
smithay = { version = "0.3", default-features = false, features = ["backend_egl", "backend_drm"] }
glow = "0.14"
drm-fourcc = "2.2"

# Async runtime
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

### 4.3 Critical Integration: Two-FBO Pipeline

The WASM runtime's FemtoVG renderer has the same Y-flip issue as the settings widget. The solution:

```rust
// In egl.rs (adapted from widgets/settings/src/egl.rs)

pub struct EglState {
    // ... same as settings widget ...
    /// Staging FBO where FemtoVG renders (regular GL texture + stencil)
    staging: Option<StagingBuffer>,
    /// Export buffers (EGLImage-backed, for DMA-BUF)
    buffers: [Option<RenderBuffer>; 2],
    /// Blit shader program (fullscreen quad with Y-flip)
    blit_program: Option<BlitResources>,
}

impl EglState {
    /// Begin frame — returns staging FBO ID for WasmWidgetRuntime
    pub fn begin_frame(&mut self) -> Result<u32> {
        // Allocate buffers if needed
        // Bind staging FBO
        // Clear
        // Return staging FBO ID
    }

    /// Blit staging → export with Y-flip (CRITICAL)
    pub fn blit_to_export(&self) -> Result<()> {
        // Bind export FBO
        // Use Y-flip shader
        // Draw fullscreen quad sampling staging texture
    }

    /// End frame — export DMA-BUF
    pub fn end_frame(&mut self) -> Result<DmaBufInfo> {
        // glFinish() to ensure rendering complete
        // Get/cache DMA-BUF fd
        // Swap buffers
    }
}
```

**Key difference from design v1:** The WASM runtime does NOT handle its own FBO creation. The widget host must:
1. Create the staging FBO with stencil (FemtoVG requirement)
2. Tell FemtoVG to render to that FBO via `set_screen_target()`
3. Blit to export FBO with Y-flip
4. Export as DMA-BUF

### 4.4 Data Flow

```
1. Startup:
   ┌─────────────────────────────────────────────────────────────────┐
   │ Read DECK_PARAMS["wasmPath"] → Load .wasm file                  │
   │ Initialize EGL context (smithay EGLDisplay from GBM)            │
   │ Create two-FBO pipeline (staging + export)                      │
   │ Create WasmWidgetRuntime with GL function loader                │
   │   - Pass staging FBO ID for FemtoVG's set_screen_target()       │
   │ Connect to Wayland, bind deck_widget_v1, get surface role       │
   └─────────────────────────────────────────────────────────────────┘

2. Render Loop (blocking_dispatch pattern from settings widget):
   ┌─────────────────────────────────────────────────────────────────┐
   │ queue.blocking_dispatch(&mut state)  // Waits for events        │
   │                                                                 │
   │ On touch_down/motion/up from deck_widget_surface_v1:            │
   │   → runtime.push_touch_event(TouchEvent { x, y, phase })        │
   │   → state.needs_render = true                                   │
   │                                                                 │
   │ On frame callback (wl_callback::Done):                          │
   │   → state.needs_render = true                                   │
   │                                                                 │
   │ If needs_render && pending_buffers < 3:                         │
   │   → egl.begin_frame() → staging FBO ID                          │
   │   → runtime.renderer().begin_frame(w, h)                        │
   │   → runtime.render(delta_ms)  // Executes WASM                  │
   │   → runtime.renderer().flush()                                  │
   │   → egl.blit_to_export()  // Y-flip copy                        │
   │   → egl.end_frame() → DmaBufInfo                                │
   │   → Create wl_buffer from DMA-BUF                               │
   │   → surface.attach(), damage_buffer(), frame(), commit()        │
   │   → pending_buffers += 1                                        │
   │                                                                 │
   │ On wl_buffer::Release:                                          │
   │   → buffer.destroy()                                            │
   │   → pending_buffers -= 1                                        │
   │                                                                 │
   │ If runtime.wants_next_frame():                                  │
   │   → needs_render = true (next iteration)                        │
   └─────────────────────────────────────────────────────────────────┘

3. Shutdown:
   ┌─────────────────────────────────────────────────────────────────┐
   │ On deck_widget_surface_v1::shutdown event:                      │
   │   → state.running = false                                       │
   │   → Loop exits, cleanup happens via Drop                        │
   └─────────────────────────────────────────────────────────────────┘
```

---

## 5. Integration Points

### 5.1 Widget Manifest

```json
{
  "uid": "550e8400-e29b-41d4-a716-446655440100",
  "version": "1.0.0",
  "name": "WASM Widget",
  "description": "WebAssembly widget runner using bmc-wasm-runtime",
  "author": {
    "name": "Braiins",
    "url": "https://braiins.com"
  },
  "binary": "bin/bmc-widget-wasm",
  "settings": [],
  "sizes": ["full"],
  "params": {
    "wasmPath": {
      "name": "WASM Path",
      "type": "string",
      "description": "Path to the .wasm file to execute",
      "default": "/mnt/data/wasm/hello_widget.wasm"
    }
  }
}
```

### 5.2 Scene Configuration

```json
{
  "scenes": [
    {
      "id": "wasm-demo-scene",
      "kind": "fullscreen",
      "widgets": [
        {
          "widget_type_id": "550e8400-e29b-41d4-a716-446655440100",
          "params": {
            "wasmPath": "/mnt/data/wasm/hello_widget.wasm",
            "testMode": false
          }
        }
      ]
    }
  ]
}
```

**Test Mode:** Set `"testMode": true` to bypass WASM loading and render test content directly. Useful for diagnosing rendering pipeline issues without requiring a valid WASM file.

### 5.3 Touch Event Mapping

```rust
// In wayland.rs
impl Dispatch<DeckWidgetSurfaceV1, ()> for WasmState {
    fn event(
        state: &mut Self,
        _: &DeckWidgetSurfaceV1,
        event: deck_widget_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            deck_widget_surface_v1::Event::TouchDown { id: _, x, y } => {
                state.runtime.push_touch_event(TouchEvent {
                    id: 0,
                    x: x as f32,
                    y: y as f32,
                    phase: TouchPhase::Start,
                });
                state.needs_render = true;
            }
            deck_widget_surface_v1::Event::TouchMotion { id: _, x, y } => {
                state.runtime.push_touch_event(TouchEvent {
                    id: 0,
                    x: x as f32,
                    y: y as f32,
                    phase: TouchPhase::Move,
                });
                state.needs_render = true;
            }
            deck_widget_surface_v1::Event::TouchUp { id: _ } => {
                state.runtime.push_touch_event(TouchEvent {
                    id: 0,
                    x: 0.0,
                    y: 0.0,
                    phase: TouchPhase::End,
                });
                state.needs_render = true;
            }
            deck_widget_surface_v1::Event::Shutdown => {
                state.running = false;
            }
            deck_widget_surface_v1::Event::Setting { setting_type, value } => {
                tracing::debug!("Setting: {:?} = {}", setting_type, value);
                // Could pass to WASM if needed
            }
            _ => {}
        }
    }
}
```

### 5.4 DMA-BUF Buffer Creation (from settings widget)

```rust
fn create_buffer_from_dmabuf(
    linux_dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    info: &DmaBufInfo,
    qh: &QueueHandle<WasmState>,
) -> wl_buffer::WlBuffer {
    let params = linux_dmabuf.create_params(qh, ());

    let modifier: u64 = info.modifier.into();
    let modifier_hi = (modifier >> 32) as u32;
    let modifier_lo = (modifier & 0xFFFF_FFFF) as u32;

    params.add(
        info.fd.as_fd(),
        0, // plane index
        0, // offset
        info.stride,
        modifier_hi,
        modifier_lo,
    );

    params.create_immed(
        info.width as i32,
        info.height as i32,
        info.format as u32,
        zwp_linux_buffer_params_v1::Flags::empty(),
        qh,
        (),
    )
}
```

---

## 6. Key Differences from Settings Widget

| Aspect | Settings Widget | WASM Widget |
|--------|-----------------|-------------|
| **Rendering** | FemtoVG directly | FemtoVG via WasmWidgetRuntime |
| **UI Logic** | Native Rust | WASM (sandboxed) |
| **Touch Handling** | Inline hit-testing | Delegated to WASM runtime |
| **Frame Requests** | Manual `needs_render` | `runtime.wants_next_frame()` |
| **State Machine** | 8-page navigation | WASM-defined |
| **EGL Setup** | Same two-FBO pipeline | Same two-FBO pipeline |
| **Buffer Management** | Same double-buffering | Same double-buffering |

---

## 7. Error Handling

### 7.1 WASM Fuel Exhaustion

```rust
match runtime.render(delta_ms) {
    Ok(()) => { /* success */ }
    Err(e) if e.to_string().contains("fuel") => {
        tracing::warn!("WASM exceeded fuel budget, skipping frame");
        // Don't crash, just skip this frame
    }
    Err(e) => {
        tracing::error!("WASM render failed: {e}");
        // Consider: restart WASM instance? display error screen?
    }
}
```

### 7.2 WASM Load Failure

```rust
fn load_wasm(path: &str) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read WASM file: {path}"))?;

    if bytes.len() < 4 || &bytes[0..4] != b"\0asm" {
        bail!("not a valid WASM file: {path}");
    }

    Ok(bytes)
}
```

### 7.3 Pending Buffer Limit

Same as settings widget — limit to 3 pending buffers to avoid GPU memory exhaustion:

```rust
if state.needs_render && state.pending_buffers < 3 {
    // Render frame
    state.pending_buffers += 1;
}
```

### 7.4 EGLImage Leak Prevention

**Critical fix:** EGLImages must be explicitly destroyed to prevent GPU virtual address space exhaustion. The `RenderBuffer` cleanup now calls `eglDestroyImageKHR`:

```rust
fn destroy_render_buffer(
    gl: &glow::Context,
    egl_destroy_image: EglDestroyImageKhr,
    egl_display_raw: *mut c_void,
    buf: RenderBuffer,
) {
    unsafe {
        gl.delete_framebuffer(buf.fbo);
        gl.delete_texture(buf.texture);
        // CRITICAL: Destroy EGLImage to free GPU VA space
        egl_destroy_image(egl_display_raw, buf.egl_image);
    }
    // GBM BO is dropped here via buf.bo Drop
}
```

Without this, each resize or buffer allocation leaks GPU VA space, eventually causing `eglCreateImageKHR` to fail.

---

## 8. Test Mode

The widget supports a test mode that bypasses WASM loading entirely, rendering test content directly using FemtoVG. This is useful for:

- Diagnosing rendering pipeline issues (EGL, DMA-BUF, Wayland)
- Verifying the two-FBO Y-flip pipeline works correctly
- Testing without requiring a valid WASM file

### 8.1 Enabling Test Mode

Three ways to enable test mode:

1. **Command line flag:** `--test`
2. **Environment variable:** `DECK_TEST_MODE=1`
3. **Scene config param:** `"testMode": true` in DECK_PARAMS

### 8.2 Test Renderer

The test renderer (`test_renderer.rs`) showcases all rendering primitives:

- Basic shapes: `fill_rect`, `fill_rounded_rect`, `fill_circle`, `stroke_rect`, `draw_line`
- Text rendering: Multiple sizes and colors
- Color palette: Grays, brand colors, alpha blending
- Animations: Bouncing circles, color cycling, pulsing, progress bars
- Transforms: `save`/`restore`, `translate`, `rotate`, `push_scissor`/`pop_scissor`

### 8.3 Logging

Widget logs are written to `/var/log/bmc/wasm-widget.log` (not stderr) for reliable debugging on the target device. Log level is controlled via `RUST_LOG` environment variable.

---

## 9. Performance Considerations

### 9.1 Memory Budget

| Component | Estimated Memory |
|-----------|------------------|
| WASM linear memory | 1-4 MB (typical widget) |
| FemtoVG atlas | 2 MB |
| cosmic-text font cache | 1-2 MB |
| GBM buffers (2x 1280x480x4) | 4.9 MB |
| **Total** | **~12 MB max** |

### 9.2 Optimizations (from settings widget)

1. **`glFinish()` in `end_frame()`** — Must wait for GPU before exporting DMA-BUF
2. **`glFlush()` not needed** — Frame callback pacing handles this
3. **DMA-BUF fd caching** — Avoid `drmPrimeHandleToFD` syscall per frame
4. **Attribute location caching** — Avoid string lookups in blit shader
5. **Double buffering** — Alternate export FBOs to avoid stalls
6. **EGLImage cleanup** — Properly destroy EGLImages to prevent GPU VA leaks

---

## 10. Implementation Plan (Complete)

### Why Settings Widget as Base (Not Flip-Clock)

| Aspect | Settings Widget | Flip-Clock Widget |
|--------|-----------------|-------------------|
| **Renderer** | FemtoVG (vector graphics) | Raw OpenGL ES |
| **FBO Pipeline** | Two-FBO with Y-flip | Single FBO (no Y-flip) |
| **Stencil Buffer** | Required (FemtoVG) | Not needed |
| **Protocol** | `deck_widget_manager_v1` | `xdg_wm_base` (desktop only) |
| **Touch Events** | Yes (via protocol) | None |
| **Buffer Tracking** | `pending_buffers` (max 3) | Simple destroy-on-release |

**Flip-clock is unsuitable** because:
1. Uses `xdg_wm_base` (standard XDG shell) for desktop testing - won't work with BMC compositor
2. No two-FBO Y-flip pipeline - FemtoVG (used by `bmc-wasm-runtime`) requires this
3. No `deck_widget_v1` protocol support - no touch events, no settings, no shutdown

### Touch Event Routing (Future Consideration)

The compositor routes touch events as follows:
- **Tap gesture** → Toggles overlay visibility (shows settings widget)
- **Drag gesture** → Scene cycling (swipe between scenes)
- **When overlay is visible** → Touch forwarded to overlay widget via `touch_down/motion/up`

For the WASM widget as a **scene widget** (not overlay):
- Taps will toggle the settings overlay (not forwarded to WASM)
- Drags will trigger scene cycling (not forwarded to WASM)
- Only overlay widgets receive touch events currently

**Phase 1 ignores touch** because scene widgets don't receive touch events anyway - the compositor intercepts them for gestures.

### Phase 1: Scaffold ✓ Complete

1. ✓ Copied `widgets/settings/` to `widgets/wasm/`
2. ✓ Removed settings-specific code (`renderer.rs`, UI state machine, hit-testing)
3. ✓ Kept: `wayland.rs` structure (protocol bindings), `egl.rs` (two-FBO pipeline)
4. ✓ Updated Cargo.toml with `bmc-wasm-runtime` dependency
5. ✓ Touch events ignored - scene widgets don't receive them

### Phase 2: WASM Integration ✓ Complete

1. ✓ Load WASM bytes from `DECK_PARAMS["wasmPath"]`
2. ✓ Create `WasmWidgetRuntime` with GL loader and FBO ID
3. ✓ Hook runtime rendering into two-FBO pipeline
4. ✓ Touch forwarding skipped (not needed for scene widgets)

### Phase 3: Testing ✓ Complete

1. ✓ Test mode implemented (`--test`, `DECK_TEST_MODE`, `testMode` param)
2. ✓ Test renderer showcases all primitives
3. ✓ Scene cycling integration works
4. ✓ `shutdown` event handling verified
5. ✓ EGLImage leak fixed (GPU VA space exhaustion)
6. ✓ Dedicated logging to `/var/log/bmc/wasm-widget.log`

### Phase 4: Touch Support (Future)

If WASM widget becomes an **overlay widget** (like settings):
1. Forward touch events to `runtime.push_touch_event()`
2. Handle velocity prediction if needed for sliders
3. Consider interaction with settings overlay (mutual exclusion)

---

## 11. Key Files for Reference

| File | Purpose |
|------|---------|
| `widgets/wasm/src/main.rs` | Entry point, logging setup, test mode detection |
| `widgets/wasm/src/wayland.rs` | Wayland client, WASM/test mode dispatch |
| `widgets/wasm/src/egl.rs` | Two-FBO pipeline with EGLImage cleanup |
| `widgets/wasm/src/test_renderer.rs` | Test mode rendering (all primitives) |
| `widgets/wasm/manifest.json` | Widget manifest (UID, params) |
| `bmc-wasm-runtime/src/runtime.rs` | WasmWidgetRuntime API |
| `bmc-wasm-runtime/src/gpu.rs` | FemtoVgRenderer implementation |
| `bmc-widget-protocol/protocol/deck-widget-v1.xml` | Protocol definition |
| `deploy_corinthia.py` | Deployment script (includes WASM widget) |

---

## 12. Appendix: Code Skeleton

### 12.1 `main.rs`

```rust
mod egl;
pub mod test_renderer;
mod wayland;

use std::fs::OpenOptions;
use std::sync::Mutex;
use anyhow::Result;
use tracing_subscriber::{EnvFilter, filter::LevelFilter, fmt, prelude::*};

const WIDGET_LOG_PATH: &str = "/var/log/bmc/wasm-widget.log";

fn main() -> Result<()> {
    // Initialize logging to a dedicated file
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(WIDGET_LOG_PATH)
        .expect("BUG: failed to open wasm-widget log file");

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive(LevelFilter::INFO.into()));

    tracing_subscriber::registry()
        .with(fmt::layer().with_ansi(false).with_writer(Mutex::new(log_file)))
        .with(filter)
        .init();

    // Check for test mode via --test flag or DECK_TEST_MODE env var
    let test_mode = std::env::args().any(|arg| arg == "--test")
        || std::env::var("DECK_TEST_MODE").is_ok_and(|v| v == "1" || v == "true");

    tracing::info!("Starting WASM widget (test_mode={})", test_mode);

    let mut client = wayland::WaylandClient::connect(test_mode)?;
    client.run()
}
```

### 12.2 `wayland.rs` (Key Differences from Settings Widget)

```rust
use bmc_wasm_runtime::WasmWidgetRuntime;

struct WasmState {
    // ... Wayland state from settings widget ...

    /// Path to WASM file
    wasm_path: String,
    /// Test mode - bypass WASM loading
    test_mode: bool,
}

impl WaylandClient {
    pub fn run(&mut self) -> Result<()> {
        if self.state.test_mode {
            self.run_test_mode()
        } else {
            self.run_wasm_mode()
        }
    }

    fn run_wasm_mode(&mut self) -> Result<()> {
        let qh = self.queue.handle();

        // Initialize EGL (two-FBO pipeline)
        let mut egl = egl::EglState::new(self.state.width, self.state.height)?;

        // Get staging FBO for WASM runtime
        let fbo_id = egl.begin_frame()?;

        // Load WASM and create runtime with FBO ID
        let wasm_bytes = std::fs::read(&self.state.wasm_path)?;
        let mut runtime = unsafe {
            WasmWidgetRuntime::new(
                &wasm_bytes,
                |symbol| smithay::backend::egl::get_proc_address(symbol),
                self.state.width,
                self.state.height,
                fbo_id,  // Pass staging FBO ID
            )?
        };

        // Main loop (same pattern as settings widget)
        while self.state.running {
            self.queue.blocking_dispatch(&mut self.state)?;

            if self.state.needs_render && self.state.pending_buffers < 3 {
                self.render_frame(&mut runtime, &mut egl, &qh)?;
            }
        }

        Ok(())
    }

    fn render_frame(
        &mut self,
        runtime: &mut WasmWidgetRuntime,
        egl: &mut egl::EglState,
        qh: &QueueHandle<WasmState>,
    ) -> Result<()> {
        self.state.needs_render = false;

        let _fbo_id = egl.begin_frame()?;

        // Render via WASM runtime
        runtime.renderer().begin_frame(self.state.width, self.state.height);
        runtime.render(16)?;  // ~16ms delta
        runtime.renderer().flush();

        // Blit staging → export with Y-flip
        egl.blit_to_export()?;
        let dmabuf_info = egl.end_frame()?;

        // Create buffer and commit
        let buffer = create_buffer_from_dmabuf(&linux_dmabuf, &dmabuf_info, qh);

        if let Some(ref surface) = self.state.surface {
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, dmabuf_info.width as i32, dmabuf_info.height as i32);
            surface.frame(qh, ());
            surface.commit();
            self.state.pending_buffers += 1;
        }

        // Check if WASM wants another frame (animation)
        if runtime.wants_next_frame() {
            self.state.needs_render = true;
        }

        Ok(())
    }
}
```

---

## 13. References

- BDK-266 branch: `jku/BDK-266-wasm`
- BDK-226 branch: `mv/BDK-226/corinthia-demo-wayland`
- `widgets/wasm/` - WASM widget implementation
- `bmc-wasm-runtime/README.md` - SDK API documentation
- `widgets/settings/` - Reference widget implementation (FemtoVG, two-FBO pipeline)
- `bmc-widget-protocol/protocol/deck-widget-v1.xml` - Wayland protocol
- `deploy_corinthia.py` - Deployment script with WASM support

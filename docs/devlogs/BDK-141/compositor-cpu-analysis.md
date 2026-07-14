# Compositor CPU Usage Analysis — Idle Scene Bottlenecks

**Ticket**: BDK-141 **Date**: 2026-02-22 (updated 2026-02-23) **Status**: Measured — S1 (idle) confirmed, remediation
planned

## Problem Statement

The EGL compositor consumes significant CPU even when no scenes are defined (idle state). This document analyzes the
architectural causes, proposes instrumentation to confirm the hypotheses, and recommends tooling for measurement.

**Target**: Reduce compositor CPU to near-zero when no scenes are defined. An idle compositor with no scenes should
sleep indefinitely, waking only on external events.

---

## 1. Architecture Overview

The compositor runs on a dedicated thread (`egl-compositor`) using Smithay's `calloop` event loop. The main loop in
`egl_compositor.rs:202-253` follows this structure on **every iteration**:

```
loop {
    1. try_recv() all pending commands           (non-blocking drain)
    2. Accept new Wayland clients                (non-blocking)
    3. Process protocol events                   (drain queues)
    4. render_scene()                            (GPU work)
    5. Calculate time, send frame callbacks       (to ALL clients)
    6. Flush clients                              (Wayland protocol I/O)
    7. event_loop.dispatch(16ms timeout)          (calloop blocks up to 16ms)
}
```

### Key observation

Steps 1-6 execute unconditionally on every loop iteration. The only throttling mechanism is the 16ms calloop dispatch
timeout at step 7. When no calloop event sources fire, the loop wakes every ~16ms and reruns all steps regardless of
whether any work is needed.

---

## 2. Identified Bottleneck Hypotheses

### H1: Unconditional render attempts (~60 times/sec)

**Location**: `egl_compositor.rs:226-231` → `scene_renderer.rs:54-168`

`render_scene()` is called every loop iteration. Its only early exit is `is_flip_pending()` at line 59. When flip is NOT
pending (i.e., the previous frame's vblank has arrived), the function does full work even with zero widgets:

01. `buffers.back_buffer()` — allocates DMA-BUF on first call, returns reference thereafter
02. `dmabuf.clone()` — clones the DMA-BUF descriptor (fd duplication or refcount)
03. `widgets.active_scene()` — returns empty scene layout
04. Texture import loop — iterates `buffers` vec, empty when no widgets (cheap)
05. `renderer.bind(&mut dmabuf)` — **EGL `eglMakeCurrent` + bind framebuffer** (GPU context switch)
06. `renderer.render()` — **begins a GPU render frame** (allocates GPU resources)
07. `frame.clear()` — **GPU clear to background color** (issues GL commands)
08. Widget loop — empty, skipped
09. `frame.finish()` — **GPU sync point** (`glFinish` or fence wait)
10. `egl.finish_rendering()` — **another EGL sync** (`eglSwapBuffers` or similar)
11. `output.page_flip(fb)` — **DRM ioctl** to schedule scanout

**Impact**: Even with zero widgets visible, the compositor performs a full GPU clear-render-sync-pageflip cycle ~30
times/sec (every other 16ms tick, alternating with flip-pending skips at ~60Hz vblank rate). On the STM32MP157's Mali
GPU, each cycle involves:

- EGL context bind/unbind
- GL clear command
- GPU-CPU synchronization fence
- DRM page flip ioctl + kernel interrupt handling

**Estimated cost**: This is likely the dominant CPU consumer. The Mali GPU driver on this SoC uses software fallbacks
and CPU-side command submission that are not free, even for a simple clear operation.

#### Deep dive: Why `renderer.bind()` is expensive on every frame

Smithay's `Renderer::bind(&mut Dmabuf)` calls `eglMakeCurrent()` internally, which performs an EGL context switch. On
desktop GPUs this is near-instant, but on embedded Mali (Panfrost/Bifrost) the cost is significant:

1. **Kernel-side context save/restore**: The Panfrost kernel driver must flush the GPU pipeline, save the current
   context state, and restore the target context. On Mali Bifrost (G31), this involves saving shader core state, tiler
   state, and L2 cache management.

2. **Cache invalidation**: Each bind triggers GPU cache invalidation. The Mali G31 has a tile-based deferred rendering
   (TBDR) architecture where the tiler and fragment shading caches must be coherent with the new render target. A bind
   to a new DMA-BUF framebuffer forces cache flushes even if it's the same buffer as last time.

3. **Mesa/Panfrost userspace overhead**: The Panfrost Mesa driver performs descriptor validation, batch object setup,
   and GEM buffer management on each `eglMakeCurrent`. This is CPU work in userspace that scales with the complexity of
   the EGL state. The Panfrost driver's dirty tracking (which improved draw performance by ~400% per Collabora's
   benchmarks) helps for repeated draws within a bound context, but does NOT help across bind/unbind cycles — each bind
   resets the dirty state.

4. **DMA-BUF import overhead**: Each `render_scene()` call does `buffer.dmabuf.clone()` followed by
   `renderer.bind(&mut dmabuf)`. Even though we're re-binding the same double-buffered render targets, Smithay's
   GlesRenderer re-imports the DMA-BUF as an EGL image and re-attaches it as a GL framebuffer color attachment. This
   involves `eglCreateImageKHR` + `glFramebufferTexture2D` calls on every frame.

**The fundamental problem**: Our compositor only has one EGL context and one render target (the double-buffered
framebuffer). There is no need to bind/unbind between frames — we could keep the context permanently current on the
compositor thread and only re-bind when the render target changes (i.e., on buffer swap). This is exactly what
production compositors like Niri do (see Section 7).

> **Update (2026-02-23)**: Measurement shows `bind` costs only ~4µs/frame on this hardware — the EGL context switch is
> essentially free. The real cost is in `finish` (GPU fence wait) at 8.2ms/frame. The bind optimization (O2) is **not
> needed**; the priority is eliminating unnecessary render cycles entirely via `needs_redraw`.

### H2: Frame callbacks sent to all clients unconditionally

**Location**: `egl_compositor.rs:241` → `state.rs:111-115`

`send_frame_callbacks()` fires every loop iteration, draining `pending_frame_callbacks`. This tells every Wayland client
"a frame was presented" — even when:

- `render_scene()` was skipped (flip pending)
- No widgets were actually composited
- The scene is empty

This causes widget processes (even hidden ones whose surfaces are committed but not in the active scene) to re-render
and submit new buffers at ~60fps, burning CPU in those processes too.

**Note**: Hidden widgets were intended to pause when the compositor stops sending frame callbacks to non-visible
widgets. However, `state.rs:200-201` collects frame callbacks from all committed surfaces into `pending_frame_callbacks`
without filtering by visibility.

### H3: Busy polling on command channel

**Location**: `egl_compositor.rs:203-205`

```rust
while let Ok(cmd) = app_state.command_rx.try_recv() {
    handle_command(&mut app_state, cmd);
}
```

This is a non-blocking drain — fine in itself, but combined with the 16ms timeout loop, it means the channel is polled
~62 times/sec even when no commands arrive. The overhead is negligible compared to H1, but it's part of the "always
spinning" pattern.

### H4: Listening socket accept() polling

**Location**: `egl_compositor.rs:212`

`listening_socket.accept()` is called every iteration. This is a non-blocking `accept4()` syscall that returns `EAGAIN`
when no clients are connecting. Individually cheap, but adds up: ~62 syscalls/sec for zero benefit when idle.

### H5: Client flush every iteration

**Location**: `egl_compositor.rs:243`

`display.flush_clients()` runs unconditionally, even when no protocol data was produced. With zero connected clients,
this is near-free. With connected but idle clients, it still performs a write syscall per client.

### H6: SystemTime::now() called every iteration

**Location**: `egl_compositor.rs:237-240`

Two syscalls per iteration (`clock_gettime` for time calculation). Individually fast (~20-50ns via vDSO), but
symptomatic of the "do everything every tick" design.

---

## 3. Bottleneck Ranking (estimated impact)

| #     | Hypothesis                         | Estimated CPU       | Confidence                                             |
| ----- | ---------------------------------- | ------------------- | ------------------------------------------------------ |
| H1    | Unconditional GPU render cycle     | **High (30-60%)**   | High — GPU driver overhead on Mali is substantial      |
| H2    | Frame callbacks to hidden widgets  | **Medium (10-30%)** | Medium — depends on number of widget processes running |
| H1+H2 | Combined render + widget re-render | **~43%**            | High — matches the prior "~43% CPU" measurement        |
| H3    | Command channel polling            | **Low (\<1%)**      | High — trivial cost                                    |
| H4    | Socket accept polling              | **Low (\<1%)**      | High — trivial cost                                    |
| H5    | Client flush                       | **Low (\<1%)**      | High — trivial cost                                    |
| H6    | Time syscalls                      | **Negligible**      | High                                                   |

**The prior ~43% measurement** refers to using "full output as damage region for all widgets" with widgets present. The
idle case (no scenes, no widgets) is a subset of H1 only: the GPU render cycle itself.

---

## 4. Instrumentation Plan

### 4.1 Tooling: `ii-stopwatch` integration

The [`ii-stopwatch`](https://gitlab.ii.zone/bos/bos-main/-/tree/master/open/utils-rs/stopwatch) crate from the BOS
project is ideal for this task:

- **Zero-cost when disabled**: All macros compile to `{}` when the `enabled` feature is off
- **No external dependencies**: Single-file crate using only `std::time`
- **Provides**: `StopWatch` (min/avg/max timing), `Every` (periodic reporting), `Jitter` (timing jitter)
- **Production-safe**: Can be left in the code permanently with the feature gate

**Integration approach**:

1. Vendor `ii-stopwatch` into `bmc-shared/` (or add as git dependency)
2. Add workspace feature `profiling` that enables `ii-stopwatch/enabled`
3. Instrument compositor loop phases with `stopwatch_start!`/`stopwatch_stop!` macros
4. Report via `tracing::info!` gated behind `Every::new(Duration::from_secs(5))`

**Note on `SystemTime` vs `Instant`**: The crate uses `SystemTime` internally. For compositor frame timing, `Instant`
would be more appropriate (monotonic, no NTP jumps). Consider a small patch to use `Instant` or wrap the measurements
accordingly. On the embedded target, the practical difference is negligible for short intervals.

### 4.2 Complementary system-level tools

| Tool                       | Purpose                             | How to use                                                              |
| -------------------------- | ----------------------------------- | ----------------------------------------------------------------------- |
| `perf top` / `perf record` | CPU profiling with call stacks      | `perf record -g -p $(pidof bmc-openwrt) -- sleep 30` then `perf report` |
| `strace -c`                | Syscall frequency analysis          | `strace -c -p $(pidof bmc-openwrt) -e trace=ioctl,write,clock_gettime`  |
| `ftrace`                   | Kernel-level DRM/GPU driver tracing | `echo 1 > /sys/kernel/debug/tracing/events/drm/enable`                  |
| `top -H`                   | Per-thread CPU usage                | Identifies `egl-compositor` thread vs tokio workers vs widget processes |
| `/proc/<pid>/stat`         | Thread CPU counters                 | Script to sample `utime`+`stime` for compositor thread                  |

### 4.3 Instrumentation points

The following stopwatches should be placed in the compositor loop (`egl_compositor.rs:202-253`):

```rust
// At function scope (before the loop):
#[cfg(feature = "profiling")]
let mut loop_w: StopWatch = Default::default();      // Full loop iteration
#[cfg(feature = "profiling")]
let mut render_w: StopWatch = Default::default();     // render_scene() total
#[cfg(feature = "profiling")]
let mut callbacks_w: StopWatch = Default::default();  // send_frame_callbacks
#[cfg(feature = "profiling")]
let mut dispatch_w: StopWatch = Default::default();   // event_loop.dispatch()
#[cfg(feature = "profiling")]
let mut every = Every::new(Duration::from_secs(5));

// Inside the loop:
loop {
    stopwatch_start!(loop_w);

    // ... commands, accept, protocol events ...

    stopwatch_start!(render_w);
    app_state.scene_renderer.render_scene(...);
    stopwatch_stop!(render_w);

    stopwatch_start!(callbacks_w);
    app_state.compositor.send_frame_callbacks(time);
    app_state.display.flush_clients();
    stopwatch_stop!(callbacks_w);

    stopwatch_start!(dispatch_w);
    event_loop.dispatch(Some(Duration::from_millis(16)), ...);
    stopwatch_stop!(dispatch_w);

    stopwatch_stop!(loop_w);

    if every_expired!(every) {
        tracing::info!(
            "compositor: loop={loop_w} render={render_w} callbacks={callbacks_w} dispatch={dispatch_w}"
        );
        loop_w.reset();
        render_w.reset();
        callbacks_w.reset();
        dispatch_w.reset();
    }
}
```

Additionally, instrument **inside** `render_scene()` (`scene_renderer.rs`):

```rust
// Separate stopwatches for:
let mut bind_w: StopWatch     // renderer.bind() — EGL context setup
let mut clear_w: StopWatch    // frame.clear() — GPU clear
let mut compose_w: StopWatch  // widget texture rendering loop
let mut finish_w: StopWatch   // frame.finish() + finish_rendering() — GPU sync
let mut flip_w: StopWatch     // page_flip() — DRM ioctl
```

This gives a complete breakdown of where time is spent within a frame.

### 4.4 Measurement scenarios

Run the instrumented build under these conditions, collecting 60 seconds of data each:

| Scenario               | Configuration                                     | What it measures                                            |
| ---------------------- | ------------------------------------------------- | ----------------------------------------------------------- |
| **S1: Pure idle**      | No scenes configured, no widgets spawned          | Baseline compositor overhead (H1) — **DONE** (section 11.1) |
| **S2: Empty scene**    | One scene defined with zero widgets               | Scene management overhead                                   |
| **S3: Single widget**  | One scene, one small widget (e.g., flip-clock)    | Single widget render cost — **DONE** (section 11.2)         |
| **S4: Full scene**     | One scene, 8 widgets (4x2 grid)                   | Full render load                                            |
| **S5: Hidden widgets** | Two scenes, 8 widgets each, only one scene active | H2 — frame callback overhead for hidden widgets             |

For each scenario, record:

- `top -H` CPU usage for `egl-compositor` thread
- Stopwatch output (loop/render/callbacks/dispatch breakdown)
- `strace -c` syscall distribution (30-second sample)

---

## 5. Expected Findings and Remediation Directions

Based on the code analysis, the instrumentation is expected to confirm:

### Finding 1: GPU render cycle dominates idle CPU

**Expected**: `render_w` shows 2-8ms per call even with empty scene (bind + clear + sync + flip). `dispatch_w` shows
~8-14ms (blocking for remaining time in 16ms budget).

**Measured (S1)**: `render_w` avg 9.17ms (higher than expected — `finish` alone is 8.2ms). `dispatch_w` avg 6.71ms. The
GPU sync fence wait is the dominant cost, not bind or clear. See section 11.1 for full breakdown.

**Remediation**: Skip the entire render pipeline when nothing has changed:

```rust
// In render_scene():
if self.output.is_flip_pending() {
    return Ok(());
}

// NEW: Skip render if scene is unchanged and no new buffers
if !self.needs_redraw {
    return Ok(());
}
```

Track a `needs_redraw` flag that is set when:

- Active scene changes (`SetActiveScene` command)
- A widget commits a new buffer (surface `commit()`)
- A widget connects or disconnects

When idle with no changes, the compositor should block entirely on `event_loop.dispatch()` with **no timeout** (or a
very long timeout), waking only on:

- DRM vblank events
- Wayland protocol activity
- Command channel activity (by registering the flume receiver's fd with calloop)

### Finding 2: Hidden widgets re-render at 60fps

**Expected**: In S5, widget processes burn CPU even when on the inactive scene.

**Remediation**: Filter frame callbacks by active scene visibility:

```rust
// In send_frame_callbacks(), only fire callbacks for visible widgets:
pub fn send_frame_callbacks_for_scene(&mut self, active_scene: &SceneLayout, time: u32) {
    // Only send frame callbacks for surfaces belonging to visible widgets
    // Hidden widget surfaces retain their pending callbacks for when they become visible
}
```

This is the intended design from the refactor plan (Stage 15) but hasn't been implemented yet.

### Finding 3: calloop dispatch timeout drives unnecessary wakeups

**Expected**: `dispatch_w` averages ~16ms (the timeout) in idle scenarios, confirming the loop runs at 60Hz regardless
of activity.

**Measured (S1)**: `dispatch_w` avg 6.71ms — less than 16ms because `render_w` consumes 9.17ms first, leaving only
~6.8ms for dispatch to sleep. Confirms the loop runs at exactly 62.5Hz (16ms/iteration) with the timeout as the only
throttle.

**Remediation**: Use event-driven wakeups instead of fixed-interval polling:

1. Register the `flume::Receiver` with calloop (flume supports `recv_deadline`, but calloop integration requires an fd —
   use a `calloop::channel` instead or a `calloop::ping` notifier)
2. Remove the 16ms timeout; let calloop block indefinitely
3. Wake on: DRM events, Wayland protocol events, or commands from main thread
4. After waking, only render if `needs_redraw` is set

This transforms the compositor from a **polling** model to a fully **event-driven** model.

---

## 6. Implementation Sequence

1. ~~**Instrument first** (this plan) — confirm hypotheses with data~~ **DONE** — S1 measured, H1 confirmed (see section
   11.1)
2. **Add `needs_redraw` flag** — skip GPU work when scene is static
3. **Filter frame callbacks** — implement Stage 15's frame callback visibility filtering
4. **Move to event-driven loop** — replace 16ms timeout with calloop event sources
5. **Re-measure** — verify CPU drops to near-zero when idle (target: ~0% idle CPU)

Each step should be a separate commit with before/after measurements.

---

## 7. Niri Compositor Reference

[Niri](https://github.com/YaLTeR/niri) is a production Wayland compositor written in Rust using Smithay. It solves
exactly the problems we face. Its architecture provides a concrete reference for how to eliminate idle CPU usage.

### 7.1 RedrawState state machine

Niri tracks each output's redraw state via a state machine:

```
              queue_redraw()         render succeeds          vblank arrives
    Idle ─────────────────► Queued ──────────────────► WaitingForVBlank ────► Idle
                                                            │                  ▲
                                                            │ queue_redraw()   │
                                                            ▼                  │
                                                       WaitingForVBlank       │
                                                       { redraw_needed }  ────┘
```

When `render_frame()` produces **no damage** (nothing changed):

```
    Queued ──(no damage)──► WaitingForEstimatedVBlank ──(timer fires)──► Idle
```

**Key insight**: The compositor **never renders** unless something actually changed. In the `Idle` state, the event loop
blocks indefinitely — zero CPU, zero GPU, zero syscalls. The state only leaves `Idle` when `queue_redraw()` is called,
which happens on:

- Surface commits (a client submitted a new buffer)
- Window management events (resize, move, focus change)
- Animation ticks (only while animations are running)
- Configuration changes

### 7.2 Damage-driven rendering

Niri uses Smithay's `DrmCompositor::render_frame()` which returns a result indicating whether the frame produced damage:

```rust
match drm_compositor.render_frame(&mut renderer, &elements, [0.; 4], flags) {
    Ok(res) => {
        if !res.is_empty {
            // Frame has damage — submit to DRM for display
            drm_compositor.queue_frame(data)?;
            output_state.redraw_state = RedrawState::WaitingForVBlank { redraw_needed: false };
        } else {
            // No damage — do NOT submit anything to DRM
            // Set timer for estimated vblank to send frame callbacks
            rv = RenderResult::NoDamage;
        }
    }
}
```

When there is no damage:

- No GPU work is performed (no bind, no clear, no flip)
- No DRM page flip ioctl is issued
- A timer is set to fire at the estimated next vblank time
- When the timer fires, frame callbacks are sent to clients, then state returns to `Idle`

This means that when nothing changes on screen, the compositor is sleeping 100% of the time between timer wakeups (once
per ~16ms for frame callbacks, then back to infinite sleep).

### 7.3 Frame callback throttling

Niri sends frame callbacks at most once per monitor refresh cycle, tracked by a sequence counter:

```rust
pub frame_callback_sequence: u32,
const FRAME_CALLBACK_THROTTLE: Option<Duration> = Some(Duration::from_millis(995));
```

A 1-second fallback timer ensures frame callbacks fire even if the compositor is completely idle, preventing clients
from starving:

```rust
// Fallback: send frame callbacks at least once per second
event_loop.insert_source(
    Timer::from_duration(Duration::from_secs(1)),
    |_, _, state| {
        state.niri.send_frame_callbacks_on_fallback_timer();
        TimeoutAction::ToDuration(Duration::from_secs(1))
    },
);
```

This is critical: without this, Wayland clients that depend on frame callbacks to drive their event loops would hang
indefinitely when the compositor stops rendering.

### 7.4 What we should adopt from Niri

| Niri pattern                    | Our current state                    | Priority                               |
| ------------------------------- | ------------------------------------ | -------------------------------------- |
| `RedrawState` machine           | No state — render every tick         | **High** — eliminates idle GPU work    |
| Damage detection before flip    | Always clear+flip                    | **High** — avoids DRM overhead         |
| Event-driven loop (no timeout)  | 16ms polling timeout                 | **Medium** — saves wakeup overhead     |
| Frame callback throttling       | Callbacks every tick to all surfaces | **High** — reduces widget CPU          |
| Estimated vblank timer          | N/A (we always flip)                 | **Medium** — needed for no-damage case |
| Fallback 1s timer for callbacks | N/A                                  | **Low** — safety net for stuck clients |

### 7.5 Key difference: Smithay's DrmCompositor

Niri uses Smithay's higher-level `DrmCompositor` abstraction which handles buffer management, damage tracking, and frame
submission automatically. Our compositor uses the lower-level `DrmDevice` + `DrmSurface` + `GlesRenderer` directly,
which means we must implement damage tracking ourselves.

Two approaches to consider:

1. **Migrate to `DrmCompositor`** — less code, automatic damage tracking, but requires refactoring the render pipeline
   to use Smithay's `RenderElement` abstraction
2. **Implement minimal damage tracking manually** — add a `needs_redraw` flag, skip render when false, keep existing
   low-level render path

Approach 2 is simpler and sufficient for our use case (embedded, fixed layout, no window management).

---

## 8. STM32MP157 Mali GPU Specifics

### 8.1 Hardware overview

- **GPU**: ARM Mali-G31 (Bifrost architecture, v7)
- **Driver**: Panfrost (open-source Mesa driver)
- **OpenGL support**: OpenGL ES 3.1, OpenGL 3.1 (non-conformant on G31)
- **Vulkan**: 1.0 (Bifrost v6+)
- **Architecture**: Tile-Based Deferred Rendering (TBDR)
- **Shader cores**: 1 execution engine (G31 is the smallest Bifrost GPU)

### 8.2 TBDR implications for compositor workloads

Mali's TBDR architecture means rendering is split into two phases:

1. **Tiling phase** (vertex/geometry): Processes all draw calls, assigns triangles to screen tiles
2. **Fragment phase**: Processes tiles one at a time, reading only the triangles in each tile

For our compositor workload (full-screen textured quads + clear):

- The tiling phase is minimal (few triangles, simple geometry)
- The fragment phase does the actual pixel work (texture sampling + blending)
- **But**: the cost of *starting* and *finishing* a frame (job submission, cache management, synchronization) is
  proportionally very high relative to the actual pixel work

This is why even a "simple clear" is expensive: the Panfrost driver must:

1. Allocate and fill a batch object (job descriptor) in kernel memory
2. Submit the job to the GPU's Job Manager via IOCTL
3. Wait for the GPU to process the job (even if it's trivial)
4. Handle the job completion interrupt
5. Clean up the batch object

### 8.3 Panfrost-specific performance characteristics

**Dirty tracking**: Panfrost implements dirty state tracking to avoid re-uploading GPU descriptors when state hasn't
changed between draw calls. This improved synthetic benchmark performance by ~400% (Collabora, 2021). However, this
optimization only works **within a single frame** — each new frame (each `renderer.render()` call) resets dirty state,
so we get no benefit from the optimization on repeated identical frames.

**Job submission overhead**: Each GPU job submission involves:

- `DRM_IOCTL_PANFROST_SUBMIT` kernel ioctl (~50-200us on ARM Cortex-A7)
- Kernel-side job chain validation
- GPU interrupt on completion (~10-50us for interrupt handling)
- Total overhead per frame: **100-500us** just for job management

**Memory management**: Each `bind()` call potentially involves GEM buffer operations. The Panfrost driver's GEM
(Graphics Execution Manager) handles buffer-object lifecycle, mapping, and cache coherency. Re-importing the same
DMA-BUF each frame incurs repeated `DRM_IOCTL_PRIME_FD_TO_HANDLE` lookups.

### 8.4 Optimization strategies specific to this hardware

**O1: Eliminate unnecessary job submissions (highest impact)**

Skip GPU work entirely when nothing changed. On the G31, each job submission costs 100-500us of CPU time. At 30fps,
that's 3-15ms/sec of pure overhead — 0.3-1.5% CPU on a single core, but the ARM Cortex-A7 cores in the STM32MP157 are
slow (~650MHz), so this represents a larger fraction of available compute.

**O2: Keep EGL context persistently bound**

Since we have exactly one GPU context and one render target, call `eglMakeCurrent()` once at startup and never unbind.
This eliminates per-frame context save/restore. If Smithay's `GlesRenderer::bind()` cannot be modified to skip redundant
binds, consider:

- Calling `bind()` once at initialization and caching the framebuffer reference
- Or patching the renderer to detect same-target rebinds as no-ops

**O3: Use `glInvalidateFramebuffer` before clear**

On TBDR GPUs, `glInvalidateFramebuffer` tells the driver it doesn't need to load the previous tile contents from memory.
Combined with `glClear`, this allows the GPU to skip the memory read phase entirely. Smithay may or may not emit this
automatically — worth verifying.

**O4: Batch widget texture imports**

Currently, widget textures are re-imported on every frame via `import_dmabuf()` / `import_shm_buffer()`. These could be
cached by widget instance ID and only re-imported when a widget commits a new buffer (signaled via the Wayland commit
callback).

**O5: Consider Vulkan for lower overhead**

The Panfrost driver supports Vulkan 1.0 on Bifrost. Vulkan's explicit command buffer model eliminates implicit
synchronization and context management overhead. A Vulkan compute pass for compositing (blit + clear) would have:

- Zero context switching overhead
- Explicit, predictable synchronization
- Pre-recorded command buffers that can be re-submitted without CPU work

This is a larger refactor but could reduce per-frame CPU cost to near-zero. Smithay does not currently support Vulkan
rendering, so this would require a custom render path.

---

## 9. Widget-Side GPU Considerations

### 9.1 Lazy EGL initialization and the GC400 context limit

The widget host uses lazy EGL initialization — `RenderState` is only created when a widget first becomes visible, not at
widget launch. This pattern is architecturally necessary due to a hardware constraint: the GC400 GPU has a **4 EGL
context limit**. Without lazy init, every WASM widget would grab a context at launch even if it sits on an inactive
scene. With 2 scenes of 4 widgets each, eager init would exhaust the limit before the compositor itself gets a context.

The lazy init does **not** cause steady-state FPS regression. Once `RenderState` is created on first visibility, it
persists for the widget's lifetime. The only cost is a one-time cold GPU init on the first frame after becoming visible,
after which performance is identical to eager init.

**Recommendation**: Keep the lazy init pattern. It is a correctness requirement, not an optimization.

### 9.2 Widget EGL resource management (`widgets/wasm/src/egl.rs`)

Three correctness and performance issues were identified in the widget-side EGL code. These are independent of the
compositor-side bottlenecks (H1-H6) but contribute to overall system CPU usage:

**EGLImage leak on resize**: `eglDestroyImageKHR` was not called when a widget resizes its surface. Each resize creates
a new EGLImage without destroying the old one. On an embedded device running continuously, this would eventually exhaust
GPU memory.

**Missing `Drop` for `EglState`**: GPU resources (EGL context, surfaces, images) were never cleaned up when a widget
process exits. This leaks GPU-side allocations that persist until the compositor process itself exits or the GPU driver
reclaims them.

**`gl.finish()` vs `gl.flush()` — 30-60ms/frame CPU blocking**: Widgets were calling `gl.finish()` (which blocks the CPU
until the GPU completes all pending work) instead of `gl.flush()` (which submits commands and returns immediately). On
the Mali G31, GPU job completion takes 30-60ms for a typical widget frame. This means each widget was blocking its CPU
thread for the full GPU render duration every frame, burning CPU on a spin-wait. With `gl.flush()`, the CPU is free to
do other work while the GPU renders, and synchronization happens implicitly at the next buffer swap.

**Impact**: The `gl.finish()` → `gl.flush()` fix alone could reduce per-widget CPU usage significantly. With 4-8 active
widgets each blocking 30-60ms/frame, this represents a substantial fraction of total system CPU time — potentially
larger than the compositor-side H1 bottleneck.

**Recommendation**: Keep all three fixes. They are orthogonal correctness/performance improvements in widget code with
no compositor rebase risk.

---

## 10. Instrumentation Status

The `ii-stopwatch` instrumentation has been implemented:

**Compositor main loop** (`egl_compositor.rs:201-287`):

- `loop_w` — full loop iteration time
- `render_w` — `render_scene()` total
- `callbacks_w` — frame callback dispatch + client flush
- `dispatch_w` — `event_loop.dispatch()` blocking time
- Reports every 5 seconds via `tracing::info!`

**Render pipeline** (`scene_renderer.rs:22-33, 113-211`):

- `bind_w` — `renderer.bind()` (EGL context + framebuffer bind)
- `clear_w` — `frame.clear()` (GL clear command)
- `compose_w` — widget texture rendering loop
- `finish_w` — `frame.finish()` + `egl.finish_rendering()` (GPU sync)
- `flip_w` — `output.page_flip()` (DRM ioctl)
- Reports every 5 seconds via `tracing::info!`

**Build**: Enable profiling with `cargo build -p bmc-openwrt --features profiling`. Without the feature flag, all
stopwatch macros compile to no-ops (zero runtime cost).

---

## 11. Measured Results

### 11.1 S1: Pure idle — no scenes, no widgets

**Environment**: STM32MP157 control board, release build with `--features profiling` **Date**: 2026-02-23 **Duration**:
60 seconds continuous sampling (12 reporting intervals at 5s each) **Configuration**: Compositor running, no scenes
defined, no widget processes

#### Raw data (representative sample, format: `count:avg/max`)

```
render_scene: bind=313:3.94µs/44µs  clear=313:401µs/1.39ms  compose=313:1.26µs/42µs
              finish=313:8.23ms/9.17ms  flip=313:325µs/1.61ms

compositor:   loop=313:16.01ms/16.71ms  render=313:9.17ms/10.95ms
              callbacks=313:13.4µs/282µs  dispatch=313:6.71ms/7.66ms
```

313 samples per 5-second window = 62.6 fps (matching ~16ms loop period).

#### Per-frame budget breakdown

| Phase                   | Avg         | Max    | % of 16ms frame | Category     |
| ----------------------- | ----------- | ------ | --------------- | ------------ |
| **finish** (GPU sync)   | **8.22ms**  | 9.64ms | **51.4%**       | GPU wait     |
| dispatch (calloop idle) | 6.71ms      | 7.97ms | 41.9%           | Idle sleep   |
| clear (GL clear)        | 0.41ms      | 4.36ms | 2.5%            | GPU work     |
| flip (DRM page flip)    | 0.34ms      | 2.10ms | 2.1%            | Kernel ioctl |
| callbacks               | 0.013ms     | 0.28ms | 0.1%            | Protocol I/O |
| bind (EGL context)      | **0.004ms** | 0.09ms | **\<0.1%**      | EGL overhead |
| compose (widget loop)   | 0.001ms     | 0.11ms | \<0.1%          | Compositing  |

#### Analysis

**H1 confirmed — GPU render cycle is the dominant cost.** The compositor spends 9.2ms/frame (57%) on GPU work that
produces an identical black frame every time. The remaining 6.7ms (42%) is idle sleep in `event_loop.dispatch()`.

**Key findings**:

1. **`finish` (GPU fence wait) is the bottleneck at 8.2ms/frame.** This is `frame.finish()` + `egl.finish_rendering()` —
   a blocking CPU wait for the Mali G31 to complete its TBDR pipeline. Even a trivial clear goes through the full tiler
   → fragment shader → writeback path. The CPU is completely idle during this wait, but the thread is blocked and cannot
   do other work.

2. **`bind` is essentially free (~4µs).** This disproves the hypothesis that `eglMakeCurrent` is expensive on this
   hardware. The EGL context switch overhead is negligible. Optimization O2 (persistent EGL bind) is **not needed**.

3. **`clear` costs ~400µs.** This is GPU job submission overhead (Panfrost `DRM_IOCTL_PANFROST_SUBMIT`) plus the GL
   clear command. Non-trivial but dwarfed by the sync.

4. **`flip` costs ~340µs.** DRM page flip ioctl + kernel-side scanout scheduling.

5. **`compose` is negligible (~1µs).** Expected with zero widgets — the texture import and render loop are empty.

6. **`callbacks` is negligible (~13µs).** No connected Wayland clients in this scenario.

7. **Total waste: ~9.2ms per 16ms frame.** The compositor burns 57% of each frame on GPU work producing a static black
   image, then sleeps for the remaining 42%. With `needs_redraw` logic, the entire 16ms would be idle sleep (or infinite
   block with event-driven wakeups).

#### Revised bottleneck ranking (measured)

| #     | Hypothesis                        | Measured impact                  | Status                             |
| ----- | --------------------------------- | -------------------------------- | ---------------------------------- |
| H1    | Unconditional GPU render          | **9.2ms/frame (57%)**            | **Confirmed** — `finish` dominates |
| H1a   | EGL bind overhead                 | **0.004ms/frame (\<0.1%)**       | **Disproved** — bind is free       |
| H1b   | GPU job submission (clear)        | **0.4ms/frame (2.5%)**           | Confirmed — moderate               |
| H1c   | DRM page flip                     | **0.3ms/frame (2.1%)**           | Confirmed — moderate               |
| H2    | Frame callbacks to hidden widgets | Not measured (S1 has no widgets) | Pending S5                         |
| H3-H6 | Polling overhead                  | Included in loop overhead        | Negligible as expected             |

#### Remediation impact estimate

Implementing `needs_redraw` (skip render when idle) would eliminate the 9.2ms of GPU work per frame. The compositor
thread would spend 100% of its time in `event_loop.dispatch()`:

- **With 16ms timeout**: loop still runs at 62.5Hz but does no GPU work — CPU usage drops to the cost of the loop
  overhead itself (H3-H6), which is negligible (\<0.1ms/iteration)
- **With event-driven wakeups (no timeout)**: the thread sleeps indefinitely when idle — **CPU drops to effectively
  zero**

The target is the second option: an idle compositor with no scenes should consume near-zero CPU, waking only when a
scene is activated, a widget connects, or a command arrives.

### 11.2 S3: Single widget — flip-clock fullscreen

**Environment**: STM32MP157 control board, release build with `--features profiling` **Date**: 2026-02-23 **Duration**:
60 seconds continuous sampling **Configuration**: One scene with flip-clock widget running fullscreen (1280x480 logical)

#### Raw data (representative sample, format: `count:avg/max`)

```
render_scene: bind=95:7.8µs/104µs  clear=95:707µs/3.51ms  compose=95:573µs/2.08ms
              finish=95:42.59ms/63.94ms  flip=95:522µs/2.74ms

compositor:   loop=200:25.26ms/69.07ms  render=200:21.44ms/65.91ms
              callbacks=200:282µs/7.15ms  dispatch=200:3.38ms/13.86ms
```

~200 loop iterations per 5s = 40fps. ~90-95 render_scene calls per 5s = ~19 actual renders (~55% of render attempts
skipped via `is_flip_pending()`).

#### Per-frame budget breakdown

| Phase                 | Avg      | Max    | % of 25ms loop           | vs S1 (idle)     |
| --------------------- | -------- | ------ | ------------------------ | ---------------- |
| **finish** (GPU sync) | **43ms** | 66ms   | **172%** (exceeds frame) | **5.2x worse**   |
| clear                 | 0.63ms   | 5.3ms  | 2.5%                     | 1.5x             |
| compose               | 0.55ms   | 3.8ms  | 2.2%                     | 550x (was 1µs)   |
| flip                  | 0.47ms   | 2.7ms  | 1.9%                     | 1.4x             |
| callbacks             | 1.2ms    | 15ms   | 4.8%                     | 92x              |
| dispatch (idle)       | 2.9ms    | 14ms   | 11.6%                    | 0.43x            |
| bind                  | 0.007ms  | 0.18ms | \<0.1%                   | ~2x (still free) |

#### Analysis

**The compositor is GPU-bound with a single fullscreen widget.** The Mali G31 takes 43ms to composite one 1280x480
texture with `Transform::_270` rotation — 2.7x longer than a 16ms frame budget. This is the hard performance ceiling of
this hardware.

**Key findings**:

1. **`finish` at 43ms is the overwhelming bottleneck.** A single fullscreen textured quad with 90° rotation saturates
   the Mali G31. The TBDR pipeline must: tile the quad, sample the source texture for every output pixel (1280x480 =
   614,400 pixels), apply rotation (which defeats tiler locality — adjacent output pixels read non-adjacent source
   rows), and write back. The G31 has only one shader core executing this workload.

2. **Effective render rate is ~19fps.** The GPU cannot finish within 16ms, so roughly every other `render_scene()` call
   is skipped via `is_flip_pending()`. The loop still runs at ~40fps (25ms period) but only half the iterations produce
   a frame.

3. **`compose` costs ~550µs.** This is the CPU-side cost of texture import + `render_texture_from_to` setup per widget.
   Modest but non-zero — would scale linearly with widget count.

4. **`callbacks` grew to 1-2ms (spikes to 15ms).** The flip-clock widget is connected as a Wayland client, receiving
   frame callbacks and generating protocol traffic. Spikes likely correspond to buffer commits or surface state updates.

5. **`dispatch` shrank to ~3ms.** Almost no idle time — the loop is dominated by GPU work.

#### Comparison with S1

| Metric          | S1 (no scenes) | S3 (flip-clock) | Interpretation                             |
| --------------- | -------------- | --------------- | ------------------------------------------ |
| Loop rate       | 62.5 fps       | 40 fps          | GPU can't sustain 60fps                    |
| Render rate     | 62.5 fps       | ~19 fps         | 55% of frames flip-pending                 |
| finish          | 8.2ms          | 43ms            | Texture composite 5.2x costlier than clear |
| Total render    | 9.2ms          | 20.5ms          | Render exceeds 16ms budget                 |
| dispatch (idle) | 6.7ms          | 2.9ms           | Almost no idle time                        |
| CPU utilization | ~57% busy      | ~88% busy       | GPU-bound, not CPU-bound                   |

#### Implications for optimization

The `needs_redraw` flag is even more valuable here. The flip-clock only updates its display once per second (digit
transition). Between updates, the compositor is re-rendering an identical frame 19 times/sec, each time waiting 43ms for
the GPU. With `needs_redraw`:

- **Between digit transitions (~1s)**: zero GPU work, compositor sleeps
- **During digit transition**: render the new frame once, flip, sleep until next update
- **Expected improvement**: from ~19 renders/sec to ~1-2 renders/sec (only on actual content change), reducing GPU
  utilization by ~90%

The rotation cost (`Transform::_270`) is a concern for future investigation. If the panel can be configured for
landscape scanout natively, the rotation overhead could be eliminated entirely.

### 11.3 S3+O1+O3: Flip-clock with glFlush + needs_redraw + texture caching

**Environment**: STM32MP157 control board, release build with `--features profiling` **Date**: 2026-02-23 **Duration**:
60 seconds continuous sampling **Configuration**: One scene with flip-clock widget running fullscreen (1280x480 logical)
**Optimizations applied**:

- `glFinish()` → `glFlush()` in `egl_context.rs` (O1: eliminate CPU stall on GPU fence)
- `needs_redraw` flag in `CompositorState` (O3: skip render when scene is static)
- Texture caching via `HashMap<ObjectId, GlesTexture>` (O4: avoid per-frame re-import)
- Frame callbacks gated on successful render (pace widgets at actual display rate)
- `render_scene` returns `bool` — `false` when skipped due to `is_flip_pending()`
- `needs_redraw = true` on any commit with frame callbacks (covers same-buffer Slint commits)

#### Raw data (steady-state, format: `count:avg/max`)

```
render_scene: bind=75:7.2µs/47µs   clear=75:700µs/2.3ms  compose=75:670µs/2.5ms
              finish=75:790µs/3.3ms  flip=75:730µs/2.3ms

compositor:   loop=398:12.5ms/22ms  render=74:3.8ms/10ms
              callbacks=74:1ms/3.6ms  dispatch=398:10.7ms/19ms
```

First 5-second window excluded (warmup — initial `finish=3.6ms/102ms` as GPU pipeline settles). 74-75 renders per
5-second window = ~15 fps (flip-clock's actual update rate). 398-403 loop iterations per window = ~80 Hz loop rate, 82%
of iterations skip rendering. Callbacks count (74) matches render count exactly — widgets paced at display rate.

#### Per-frame budget breakdown (render frames only)

| Phase                   | Avg        | Max    | vs S3 baseline | vs S1 (idle)   |
| ----------------------- | ---------- | ------ | -------------- | -------------- |
| **finish** (GPU submit) | **0.79ms** | 3.3ms  | **54x better** | 10x better     |
| clear                   | 0.70ms     | 2.3ms  | ~same          | ~2x            |
| compose                 | 0.67ms     | 2.5ms  | ~same          | 670x (was 1µs) |
| flip                    | 0.73ms     | 2.3ms  | ~same          | ~2x            |
| bind                    | 0.007ms    | 0.05ms | ~same          | ~same          |
| **Total render**        | **3.8ms**  | 10ms   | **11x better** | 2.4x better    |

#### Loop budget breakdown (all iterations)

| Phase           | Avg    | Max   | vs S3 baseline           |
| --------------- | ------ | ----- | ------------------------ |
| loop (total)    | 12.5ms | 22ms  | 2x faster (was 25ms)     |
| render          | 3.8ms  | 10ms  | 5.6x better (was 21.4ms) |
| callbacks       | 1.0ms  | 3.6ms | ~same (was 1.2ms)        |
| dispatch (idle) | 10.7ms | 19ms  | 3.7x more idle time      |

#### Analysis

**Both optimizations validated — compositor CPU usage reduced by an order of magnitude.**

1. **`glFlush()` is the dominant win.** `finish` dropped from 43ms to 0.79ms (54x). Instead of blocking the CPU thread
   until the Mali G31 completes its entire TBDR pipeline, `glFlush()` just submits the GL command buffer and returns
   immediately. Buffer safety is guaranteed by implicit DMA-BUF fencing — the DRM page flip ioctl waits for the GPU to
   complete before scanout. The CPU thread is now free during GPU rendering.

2. **`needs_redraw` eliminates redundant renders.** 74-75 renders per 5-second window vs ~95 in S3 baseline. The
   flip-clock submits ~15 buffers/sec, and the compositor renders exactly once per new buffer. Between updates, the
   compositor sleeps in `event_loop.dispatch()`.

3. **Idle time increased from 12% to 86% of loop budget.** `dispatch` now consumes 10.7ms out of 12.5ms per loop
   iteration — the thread is mostly sleeping. In S3 baseline, the thread was 88% busy with GPU work.

4. **Texture caching contributes minimally for this workload.** The flip-clock submits a new DMA-BUF every frame (clock
   digits change), so the texture cache has 0% hit rate. The caching benefit would appear with multi-widget scenes where
   only one widget updates while others remain static.

5. **Frame callback pacing is critical.** Three iterations were needed to get this right:

   - **v1 (callbacks inside `if needs_render`)**: Deadlocked. Slint widgets wait for a frame callback before submitting
     the next frame. Without callbacks, no commits, no `needs_redraw`, no renders — permanent stall after initial 2
     frames.

   - **v2 (unconditional callbacks every loop iteration)**: Overflow. Widgets were paced at the 80 Hz loop rate instead
     of the ~15 Hz display rate. The flip-clock rendered at 80 fps, flooding the Wayland protocol with buffer commits.
     `callbacks` cost grew from 150µs to 4-6ms mean / 100ms max, degrading overall loop performance.

   - **v3 (callbacks gated on successful render)**: Correct. `render_scene()` returns `bool` indicating whether a frame
     was actually produced (not skipped by `is_flip_pending()`). Callbacks are sent only after a real render, pacing
     widgets at the actual display rate. `needs_redraw` stays set when render is skipped, ensuring the next attempt
     after the flip completes will render and send callbacks.

6. **`needs_redraw` must trigger on frame callbacks, not just `NewBuffer`.** Slint widgets may render to the same buffer
   without re-attaching — the commit has frame callbacks but no `BufferAssignment::NewBuffer`. Setting
   `needs_redraw = true` when `frame_callbacks` is non-empty ensures all visual updates trigger a render, regardless of
   buffer attachment pattern.

#### Comparison with previous measurements

| Metric              | S1 (idle) | S3 (baseline)      | S3+O1+O3            | Target           |
| ------------------- | --------- | ------------------ | ------------------- | ---------------- |
| Loop rate           | 62.5 fps  | 40 fps             | 80 fps              | Event-driven     |
| Render rate         | 62.5 fps  | ~19 fps            | **15 fps**          | Content-driven   |
| `finish` per render | 8.2ms     | 43ms               | **0.79ms**          | \<1ms            |
| Total render        | 9.2ms     | 20.5ms             | **3.8ms**           | \<5ms            |
| Dispatch (idle)     | 6.7ms     | 2.9ms              | **10.7ms**          | Infinite (event) |
| CPU utilization     | ~57%      | ~88%               | **~14%**            | ~0% idle         |
| Renders skipped     | 0%        | 55% (flip-pending) | **82%** (no redraw) | >95%             |

#### Remaining optimization opportunities

1. **Event-driven wakeups**: Replace the 16ms dispatch timeout with `calloop` event-driven wakeups (wake on Wayland fd
   activity, DRM vblank, or command channel). This would reduce the 80 Hz idle polling to zero, making the compositor
   truly sleep when idle.

2. **Rotation elimination**: If the display panel can be configured for landscape scanout natively (via DRM CRTC
   rotation or panel orientation property), the `Transform::_270` rotation in the GPU composite pass could be
   eliminated, further reducing GPU render time.

---

## 12. References

- `bmc-openwrt/src/compositor/egl_compositor.rs` — main compositor loop
- `bmc-openwrt/src/compositor/scene_renderer.rs` — render pipeline
- `bmc-openwrt/src/compositor/render/drm_output.rs` — DRM page flip mechanism
- `bmc-openwrt/src/compositor/state.rs` — frame callback dispatch, surface commit handling
- `widgets/wasm/src/egl.rs` — widget-side EGL resource management (lazy init, cleanup, sync)
- [`ii-stopwatch`](https://gitlab.ii.zone/bos/bos-main/-/tree/master/open/utils-rs/stopwatch) — zero-cost timing
  instrumentation crate
- [Niri compositor](https://github.com/YaLTeR/niri) — reference Smithay-based Wayland compositor
- [Niri Redraw Loop wiki](https://github.com/YaLTeR/niri/wiki/Development:-Redraw-Loop) — documentation of Niri's
  damage-driven rendering state machine
- [Panfrost Mesa driver](https://docs.mesa3d.org/drivers/panfrost.html) — Mali GPU open-source driver documentation
- [Panfrost dirty tracking (Collabora)](https://www.collabora.com/news-and-blog/blog/2021/06/11/open-source-opengl-es-3.1-on-mali-gpus-with-panfrost/)
  — Panfrost OpenGL ES 3.1 + performance optimization details
- [Introduction to damage tracking (emersion)](https://emersion.fr/blog/2019/intro-to-damage-tracking/) — Wayland
  compositor damage tracking concepts

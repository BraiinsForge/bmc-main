# System overlays 1c — hosted entrypoint + bmc-wasm-host integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run system overlays *inside* `bmc-wasm-host`, sharing the host's `FemtoVgRenderer` (the memory win), driven by
the host main loop. Prove it by running the validation overlay hosted in-process (not standalone) against the
compositor.

**Architecture:** The framework gains a `HostedOverlay` bundle (the `dyn SystemOverlay` + its own `LayerSurfaceClient` +
its own `OverlayRenderTarget` — double-buffer + `wl_buffer` cache + release tracking) that opens its **own** Wayland
connection to the compositor and allocates export buffers from the host's `EglContext`. The host registers a compiled-in
list of overlays, adds their connection fds to its `poll(2)` set, ticks them each pass, and renders the ones that want
it through the shared renderer — orchestrating the frame exactly like `WidgetSlot::render` (GPU render lock → staging
FBO → `overlay.render` → blit → `flush_and_wait_gl` → `export_and_swap` → attach). The overlay borrows the parked
`NonNull<dyn Renderer>` only inside its render call.

**Tech Stack:** Rust, `bmc-wasm-host`, `bmc-system-overlay`, EGL/GLES. `bmc-wasm-host` is built/checked with
`nix develop .#ci -c cargo … -p bmc-wasm-host`.

**Context the executor needs:**

- Sub-plan 1c of BDK-416. Spec: `docs/superpowers/specs/2026-06-07-system-overlays-design.md`. Depends on 1a
  (compositor) and 1b (framework + standalone + `ValidationOverlay`).
- Mirror `bmc-wasm-host/src/slot.rs::render` (the export pipeline) and `bmc-wasm-host/src/main_loop.rs`
  (`run_with_slots`, `run_loop`, the poll set, the render pass, `compute_poll_timeout`).
- The host owns the GPU stack (`SharedHost`: `egl`, `scratch`, the GPU render lock via `acquire_gpu_render_lock`,
  `flush_and_wait_gl`, `blit_staging_to`). Overlays must ride that same path — they add no new GL context in hosted
  mode.
- Run cargo sandboxed; `nix fmt` (plain) before each commit; no ticket IDs in comments; `#[expect]` not `#[allow]`.

---

## File Structure

- Modify `system-overlays/bmc-system-overlay/src/lib.rs` — export `HostedOverlay` and a `validation` module.
- Create `system-overlays/bmc-system-overlay/src/hosted.rs` — the `HostedOverlay` bundle + accessors (no GPU
  orchestration; the host drives it).
- Move `ValidationOverlay` into `system-overlays/bmc-system-overlay/src/validation.rs` (so both the standalone bin and
  the host can construct it); update `validation-overlay/src/main.rs` to use it.
- Create `bmc-wasm-host/src/overlays.rs` — the compiled-in overlay registry + the host-side render orchestration
  (`render_hosted_overlay`).
- Modify `bmc-wasm-host/src/main_loop.rs` — construct overlays in `run`/`run_with_slots`, thread `&mut overlays` through
  `run_with_slots` → `run_loop`, add their fds to the poll set, tick + render them each pass, include them in the poll
  timeout. (The binary `main.rs` only calls `run`; it needs no change — the registry lives entirely inside the lib's
  loop functions.)
- Modify `bmc-wasm-host/src/lib.rs` — add `mod overlays;` (the crate root for `main_loop.rs`/`slot.rs`/`host.rs`).
- Modify `bmc-wasm-host/src/slot.rs` — `normalize_gl_state` → `pub(crate)`.
- Modify `bmc-wasm-host/Cargo.toml` — depend on `bmc-system-overlay`.

---

## Task 1: `HostedOverlay` bundle in the framework

**Files:**

- Create: `system-overlays/bmc-system-overlay/src/hosted.rs`

- Modify: `system-overlays/bmc-system-overlay/src/lib.rs`

- Create: `system-overlays/bmc-system-overlay/src/validation.rs`

- Modify: `system-overlays/validation-overlay/src/main.rs`

- [ ] **Step 1: Move `ValidationOverlay` into the framework**

Create `src/validation.rs` with the `ValidationOverlay` struct from plan 1b (the `impl SystemOverlay`), made `pub`. Add
`pub mod validation;` to `lib.rs` and `pub use validation::ValidationOverlay;`. Change `validation-overlay/src/main.rs`
to:

```rust
use bmc_system_overlay::{run_standalone, ValidationOverlay};

fn main() -> anyhow::Result<()> {
    run_standalone(Box::new(ValidationOverlay::default()))
}
```

- [ ] **Step 2: Write the `HostedOverlay` bundle**

`src/hosted.rs`:

```rust
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use bmc_widget::egl::{DmaBufInfo, EglContext};

use crate::gpu::OverlayRenderTarget;
use crate::overlay::{LayerConfig, SystemOverlay};
use crate::surface::LayerSurfaceClient;

const MIN_INTER_FRAME: Duration = Duration::from_millis(8);

/// A system overlay hosted inside another process (e.g. bmc-wasm-host). It owns
/// its Wayland connection and export buffers but borrows the host's renderer
/// and GPU stack for the actual frame, which the host orchestrates.
pub struct HostedOverlay {
    overlay: Box<dyn SystemOverlay>,
    client: LayerSurfaceClient,
    target: OverlayRenderTarget,
    size: (u32, u32),
    last_render: Option<Instant>,
    next_wake: Option<Instant>,
    wants_render: bool,
    /// Set after a non-fatal render/export/attach error. A failed overlay is
    /// dropped from the host's list (terminal) — it must NOT keep `wants_render`
    /// latched, or it would busy-retry-and-log every pass.
    failed: bool,
}

impl HostedOverlay {
    /// Connect the overlay's own Wayland client and allocate its export buffers
    /// from the host's EGL context.
    pub fn connect(mut overlay: Box<dyn SystemOverlay>, egl: &EglContext) -> anyhow::Result<Self> {
        let config: LayerConfig = overlay.layer_config();
        let client = LayerSurfaceClient::connect(&config)?;
        let (cw, ch) = client.size();
        let size = (
            if cw == 0 { config.size.0.max(1) } else { cw },
            if ch == 0 { config.size.1.max(1) } else { ch },
        );
        let target = OverlayRenderTarget::new(egl, size.0, size.1)?;
        overlay.init();
        Ok(Self {
            overlay,
            client,
            target,
            size,
            last_render: None,
            next_wake: None,
            wants_render: false,
            failed: false,
        })
    }

    #[must_use]
    pub fn connection_fd(&self) -> RawFd {
        self.client.connection_fd()
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Drain Wayland events, deliver touch, and pick up surface-dirty.
    pub fn dispatch(&mut self) -> anyhow::Result<()> {
        // Non-blocking: the host already polled the fd. poll_dispatch(0) runs the
        // correct prepare_read -> poll(0) -> read -> dispatch sequence, so events
        // are actually read (a bare dispatch_pending after an external poll would
        // not read the fd). See 1b Task 3 Step 5.
        self.client.poll_dispatch(0)?;
        for ev in self.client.drain_touch() {
            self.overlay.on_touch(ev);
        }
        if self.client.take_needs_render() {
            self.wants_render = true;
        }
        // Reclaim export slots the compositor released, so they are reusable.
        for released in self.client.drain_released_buffers() {
            self.target.mark_released_buffer(&released);
        }
        Ok(())
    }

    /// Run background work; updates the next-wake hint.
    pub fn tick(&mut self, now: Instant) {
        let outcome = self.overlay.tick(now);
        if outcome.wants_render {
            self.wants_render = true;
        }
        self.next_wake = outcome.next_wake;
    }

    /// Whether the overlay should be rendered this pass (respecting the
    /// inter-frame floor).
    #[must_use]
    pub fn needs_render(&self, now: Instant) -> bool {
        let inter_frame_ok = self
            .last_render
            .map_or(true, |t| now.duration_since(t) >= MIN_INTER_FRAME);
        // `target.available()` gates on a released export slot, so the host
        // never draws into a buffer the compositor is still displaying.
        !self.failed
            && self.wants_render
            && inter_frame_ok
            && self.client.running()
            && self.target.available()
    }

    /// Whether this overlay has hit a terminal non-fatal error and should be
    /// dropped from the host's list.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// Mark a terminal non-fatal failure (render/export/attach error that is
    /// not a lost GPU context). Clears the latched render so it cannot retry.
    pub fn mark_failed(&mut self) {
        self.failed = true;
        self.wants_render = false;
    }

    /// Release GL/EGL/GBM resources before this overlay is dropped. Must be
    /// called for failed overlays, closed overlays, and at host shutdown —
    /// `OverlayRenderTarget`/`DoubleBufferState` do not free on Drop.
    pub fn shutdown(&mut self, egl: &EglContext) {
        self.target.destroy(egl);
    }

    #[must_use]
    pub fn next_wake(&self) -> Option<Instant> {
        self.next_wake
    }

    /// Max time the host may sleep on this overlay's behalf (its fd is also in
    /// the poll set). `Some(ZERO)` means poll immediately. Crucially this covers
    /// the throttled case: a render latched in `wants_render` just after a frame
    /// is gated out of `needs_render` by the inter-frame floor, so without this
    /// the host could sleep on the tick wake (or forever) and never wake at the
    /// 8 ms boundary to run it.
    #[must_use]
    pub fn poll_timeout(&self, now: Instant) -> Option<Duration> {
        let tick = self.next_wake.map(|t| t.saturating_duration_since(now));
        if self.failed || !self.wants_render || !self.client.running() {
            return tick;
        }
        // A render is latched. Three cases:
        let inter_frame_remaining = self
            .last_render
            .and_then(|t| MIN_INTER_FRAME.checked_sub(now.duration_since(t)))
            .filter(|d| !d.is_zero());
        match inter_frame_remaining {
            // Throttled: wake when the inter-frame floor expires (or a sooner tick).
            Some(d) => Some(tick.map_or(d, |t| d.min(t))),
            // Not throttled and a buffer slot is free: renderable now → poll(0).
            None if self.target.available() => Some(Duration::ZERO),
            // Not throttled but blocked on a free buffer slot: the incoming
            // wl_buffer.release wakes the host via this overlay's fd; honor tick.
            None => tick,
        }
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.client.running()
    }

    // --- Accessors the host's render orchestration needs ---

    pub fn overlay_mut(&mut self) -> &mut dyn SystemOverlay {
        &mut *self.overlay
    }

    pub fn target_mut(&mut self) -> &mut OverlayRenderTarget {
        &mut self.target
    }

    /// Attach an exported buffer: mint+cache its `wl_buffer`, commit it to the
    /// layer surface, and mark the slot in-flight until the compositor releases
    /// it. Borrows target and client together (legal as one `&mut self`).
    pub fn submit_exported(&mut self, dmabuf: &DmaBufInfo, slot: usize) -> anyhow::Result<()> {
        let wl_buffer = self.target.wl_buffer_for_slot(&mut self.client, dmabuf, slot)?;
        self.client.submit_buffer_with_wl_buffer(dmabuf, &wl_buffer)?;
        self.client.flush()?;
        self.target.mark_presented(slot);
        Ok(())
    }

    /// Mark a render as completed at `now` and clear the dirty flag.
    pub fn mark_rendered(&mut self, now: Instant) {
        self.last_render = Some(now);
        self.wants_render = false;
    }
}
```

Add to `lib.rs`: `mod hosted; pub use hosted::HostedOverlay;`. Ensure `LayerSurfaceClient` and its accessors (`size`,
`running`, `take_needs_render`, `drain_touch`, `drain_released_buffers`, `poll_dispatch`, `connection_fd`,
`mint_wl_buffer`, `submit_buffer_with_wl_buffer`, `flush`) are `pub` (plan 1b Task 3 created them; widen visibility if
needed).

**Connection in hosted mode (verified):** `LayerSurfaceClient::connect` uses `Connection::connect_to_env()`.
`bmc-wasm-thin` spawns the host with `fork`+`execv`/`execvp` (spawn.rs:356/358) — not `execve` with a custom envp — so
the host **inherits the thin client's environment**, including `WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR` (the thin client
reads them in `wayland_fd.rs:11`). So `connect_to_env()` resolves the compositor socket in hosted mode; no extra wiring
is needed. `build_overlays` already degrades gracefully if the connect fails (it logs and runs with zero overlays). (A
`connect_to_path(socket)` variant is intentionally **not** added — there is no socket-path source feeding it, so it
would be dead code; reinstate only if the on-device env check below actually fails.)

- [ ] **Step 3: Build the framework**

Run: `nix develop -c cargo build -p bmc-system-overlay -p validation-overlay` Expected: PASS.

- [ ] **Step 4: Commit**

```bash
nix fmt
git add system-overlays
git commit -F - <<'EOF'
system-overlays: Add hosted overlay bundle #BDK-416

- add HostedOverlay owning its connection and export buffers while
  borrowing the host renderer, with tick/dispatch/needs_render and
  accessors for host-driven rendering
- move ValidationOverlay into the framework so the host can host it
EOF
```

---

## Task 2: Host-side overlay registry and render orchestration

**Files:**

- Create: `bmc-wasm-host/src/overlays.rs`

- Modify: `bmc-wasm-host/Cargo.toml`

- Modify: `bmc-wasm-host/src/lib.rs` — add `mod overlays;` here (this is the crate root that
  `main_loop.rs`/`slot.rs`/`host.rs` belong to; `overlays.rs` uses `crate::host`/`crate::slot` and Task 3 uses
  `crate::overlays`). Not the binary `main.rs`.

- Modify: `bmc-wasm-host/src/slot.rs` — change `normalize_gl_state` (slot.rs:1029) from private to `pub(crate)` so the
  overlay render path can reuse it as `crate::slot::normalize_gl_state`.

- [ ] **Step 1: Depend on the framework**

In `bmc-wasm-host/Cargo.toml` add `bmc-system-overlay.workspace = true`.

- [ ] **Step 2: Registry + orchestration**

`bmc-wasm-host/src/overlays.rs`:

```rust
use std::ptr::NonNull;
use std::time::Instant;

use bmc_render::renderer::Renderer;
use bmc_system_overlay::{HostedOverlay, ValidationOverlay};
use bmc_widget::egl::EglContext;

use crate::host::SharedHost;

/// Build the compiled-in system overlays. For now: the validation overlay.
/// Each opens its own Wayland connection and allocates buffers from `egl`.
pub fn build_overlays(egl: &EglContext) -> Vec<HostedOverlay> {
    let mut overlays = Vec::new();
    match HostedOverlay::connect(Box::new(ValidationOverlay::default()), egl) {
        Ok(o) => overlays.push(o),
        Err(e) => tracing::error!("failed to start validation overlay: {e}"),
    }
    overlays
}

/// Render one hosted overlay through the shared renderer, mirroring
/// `WidgetSlot::render`: lock GPU, stage, draw, blit, fence-wait, export, attach.
pub fn render_hosted_overlay(
    overlay: &mut HostedOverlay,
    ptr: NonNull<dyn Renderer>,
    shared: &mut SharedHost,
    now: Instant,
) -> anyhow::Result<()> {
    let size = overlay.size();
    let (dmabuf, slot) = {
        // Lock lifetime matches WidgetSlot::render (slot.rs:611): held across
        // render+blit+fence-wait, then dropped BEFORE export_and_swap. A bare
        // `let _lock = ...` would drop at block end (after export), holding the
        // lock across the handoff — so bind it and `drop` it explicitly.
        let gpu_render_lock = shared.acquire_gpu_render_lock("host_system_overlay")?;
        overlay.target_mut().ensure_current(&shared.egl)?;
        let _staging = shared.scratch.begin_frame(&shared.egl, size.0, size.1);
        crate::slot::normalize_gl_state(&shared.egl, size.0, size.1);

        // SAFETY: the pointer was NonNull when parked by the main loop; the
        // reborrow is valid for this call only and not stored.
        let renderer = unsafe { ptr.as_ptr().as_mut() }
            .expect("BUG: parked renderer pointer must reborrow non-null");
        renderer.begin_frame(size.0, size.1, 1.0);
        // Both scratch.begin_frame and FemtoVgRenderer::begin_frame clear opaque
        // black; a see-through overlay must start transparent. Re-clear to alpha
        // 0 AFTER femtovg's clear and before drawing.
        unsafe {
            use glow::HasContext as _;
            let gl = shared.egl.gl();
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        overlay.overlay_mut().render(renderer, size);
        renderer.flush();

        let fbo = overlay.target_mut().current_fbo();
        shared.blit_staging_to(fbo, size.0, size.1);
        shared.flush_and_wait_gl();
        drop(gpu_render_lock); // release before the buffer handoff, like slot.rs:611
        overlay.target_mut().export_and_swap()?
    };
    // Mint+attach the wl_buffer and mark the slot in-flight. Done inside one
    // HostedOverlay method so target and client are borrowed together legally.
    overlay.submit_exported(&dmabuf, slot)?;
    overlay.mark_rendered(now);
    Ok(())
}
```

`normalize_gl_state` is a private free fn in `slot.rs:1029`. Make it `pub(crate)` (a one-line edit listed in this task's
Files) and call it as `crate::slot::normalize_gl_state`. The other calls — `shared.acquire_gpu_render_lock`,
`shared.scratch.begin_frame`, `shared.blit_staging_to`, `shared.flush_and_wait_gl`, `shared.egl`, and the
`OverlayRenderTarget` methods (wrapping `DoubleBufferState`'s `ensure_current`/`current_ref().fbo`/`export_and_swap`) —
reuse the identical calls from `slot.rs::render`/`host.rs`.

- [ ] **Step 3: Register the module**

Add `mod overlays;` to `bmc-wasm-host/src/lib.rs` (the crate root; `main_loop.rs` is part of this lib, and `overlays.rs`
references `crate::host`/`crate::slot`). Do **not** put it in the binary `main.rs` — that's a separate crate root and
`crate::overlays` would not resolve from `main_loop.rs`.

- [ ] **Step 4: Build**

Run: `nix develop .#ci -c cargo build -p bmc-wasm-host` Expected: PASS (overlays module compiles; not yet wired into the
loop).

- [ ] **Step 5: Commit**

```bash
nix fmt
git add bmc-wasm-host/src/overlays.rs bmc-wasm-host/Cargo.toml bmc-wasm-host/src/lib.rs bmc-wasm-host/src/slot.rs
git commit -F - <<'EOF'
bmc-wasm-host: Add system-overlay registry and render path #BDK-416

- build the compiled-in overlay list (the validation overlay for now)
- render a hosted overlay through the shared renderer under the GPU
  render lock with the GL-fence handoff, mirroring widget slot rendering
- expose normalize_gl_state as pub(crate) for reuse
EOF
```

---

## Task 3: Wire overlays into the main loop

**Files:**

- Modify: `bmc-wasm-host/src/main_loop.rs`

- [ ] **Step 1: Construct overlays at loop start**

Build the overlays in `run_with_slots` (main_loop.rs:431) and thread `&mut overlays` into `run_loop` — extend **both**
`run_with_slots` and `run_loop` signatures (the call chain is `run` → `run_with_slots` → `run_loop`), not just
`run_loop`. Tear overlays down in `run_with_slots` **after** `run_loop` returns, mirroring the existing slot teardown
(`drain_if_err`/`drain_and_shutdown`, main_loop.rs:259). This single caller-side teardown covers every exit path —
normal idle-exit *and* the fatal early-`return Err` inside `run_loop` (Step 4) — because control returns here either
way:

```rust
    let mut overlays = crate::overlays::build_overlays(&shared.egl);
    let result = run_loop(shared, renderer, &listener, &mut slots, &mut overlays);
    for o in overlays.iter_mut() {
        o.shutdown(&shared.egl); // immutable borrow; released before drain_if_err's &mut shared
    }
    drain_if_err(result, &mut slots, shared, renderer) // existing slot teardown
```

Because teardown lives in the caller, do **not** add overlay cleanup inside `run_loop`, and the fatal
`return Err(FatalError::EglContextLost)` in Step 4 needs no special handling — it simply returns to here. (The per-pass
drain of *individual* failed/closed overlays in Step 4 is separate and stays — that handles one overlay dying mid-run,
not host exit.)

- [ ] **Step 2: Add overlay fds to the poll set**

In `run_loop` where pollfds are assembled (main_loop.rs:297-314), after adding listener + per-slot fds, append one
`pollfd` per overlay using `overlay.connection_fd()` with `POLLIN`. No per-overlay `revents` index map is needed — Step
3 dispatches overlays unconditionally, mirroring the slot loop, which also dispatches without inspecting per-slot
`revents` (main_loop.rs:356-371). The fds are added only so an incoming Wayland event (e.g. `wl_buffer.release`) wakes
`poll`.

- [ ] **Step 3: Dispatch overlay Wayland events**

After the slot dispatch block (main_loop.rs:356-371), add:

```rust
        for overlay in overlays.iter_mut() {
            if let Err(e) = overlay.dispatch() {
                // A persistent dispatch error that never delivers Closed would
                // otherwise log every pass. Treat it as terminal; the drain in
                // Step 4 drops + cleans it up.
                tracing::error!("overlay dispatch error, dropping overlay: {e}");
                overlay.mark_failed();
            }
        }
```

(Dispatch unconditionally each wake; `poll_dispatch(0)` is cheap when the fd had no events. Optionally gate on the
overlay's `revents` like slots do.)

- [ ] **Step 4: Tick + render overlays after the widget render pass**

After the slot render pass (main_loop.rs:373-400), add an overlay pass:

```rust
        let now = Instant::now();
        for overlay in overlays.iter_mut() {
            overlay.tick(now);
            if overlay.needs_render(now) {
                if let Err(e) =
                    crate::overlays::render_hosted_overlay(overlay, renderer_ptr, shared, now)
                {
                    // Mirror the slot render-error path (main_loop.rs:394): a lost
                    // EGL context is fatal and must propagate, not be swallowed
                    // until the next widget render notices it.
                    if shared.is_context_lost() {
                        return Err(FatalError::EglContextLost);
                    }
                    // Non-fatal failure: mark terminal so it is dropped below and
                    // does not busy-retry (wants_render would otherwise stay set).
                    tracing::error!("overlay render error, dropping overlay: {e}");
                    overlay.mark_failed();
                }
            }
        }
        // Drop overlays whose client closed or that hit a terminal error,
        // shutting down each first so its GPU resources are freed. A plain
        // `retain` would Drop them without `destroy(egl)` and leak.
        let mut idx = 0;
        while idx < overlays.len() {
            if !overlays[idx].running() || overlays[idx].is_failed() {
                overlays[idx].shutdown(&shared.egl);
                overlays.remove(idx);
            } else {
                idx += 1;
            }
        }
```

Confirm the exact `FatalError` variant against `slot.rs`/`main_loop.rs:394`; reuse it rather than inventing one.

- [ ] **Step 5: Include overlays in the poll timeout**

Fold each overlay's `poll_timeout(now)` (Task 1) into the slot-based timeout from `compute_poll_timeout`
(main_loop.rs:100-161). **Beware the `-1` sentinel:** `compute_poll_timeout` returns `-1` for "block forever"
(main_loop.rs:151); a numeric `min(-1, d)` is always `-1` and would silently disable every overlay wake-up (the overlay
would then only ever render when a widget animation/touch happens to wake the loop). Guard it explicitly — work in
`Option<Duration>` ("none" = block), not the raw `-1`:

```rust
    // Slot timeout as an Option<Duration>: -1 (block) -> None. The checked
    // conversion subsumes the >= 0 guard and avoids clippy::cast_sign_loss.
    let slot_ms = compute_poll_timeout(&slots, &lifetime, now);
    let slot_timeout = u64::try_from(slot_ms).ok().map(Duration::from_millis);

    // Combine with overlays. poll_timeout(now) already yields Some(ZERO) when an
    // overlay must run now (incl. the inter-frame-throttled case).
    let mut wake = slot_timeout;
    for o in overlays.iter() {
        if let Some(d) = o.poll_timeout(now) {
            wake = Some(wake.map_or(d, |w| w.min(d)));
        }
    }
    let timeout_ms = wake.map_or(-1, |d| i32::try_from(d.as_millis()).unwrap_or(i32::MAX));
```

So: `-1` only when neither slots nor any overlay want a wake; `0` when an overlay is renderable (or throttled and due
imminently); otherwise the soonest finite deadline. This closes the "latched render throttled by `MIN_INTER_FRAME` with
no wake" stall — `poll_timeout` returns the inter-frame remainder in that case.

- [ ] **Step 6: Build**

Run: `nix develop .#ci -c cargo build -p bmc-wasm-host` Expected: PASS.

- [ ] **Step 7: Commit**

```bash
nix fmt
git add bmc-wasm-host/src/main_loop.rs
git commit -F - <<'EOF'
bmc-wasm-host: Drive system overlays from the main loop #BDK-416

- build hosted overlays at startup and poll their connection fds
- dispatch overlay events, tick them, and render those that want it
  after the widget render pass, sharing the host renderer
- fold overlay wake hints into the poll timeout
EOF
```

---

## Task 4: Clippy

**Files:** none (verification)

- [ ] **Step 1: Workspace clippy**

Run: `nix develop .#ci -c cargo clippy --workspace --tests -- -D warnings` Expected: PASS, no warnings. Fix any lints in
the touched files (use `#[expect(..., reason = "…")]`, no ticket IDs in the reason).

- [ ] **Step 2: Format**

Run: `nix fmt` Expected: no changes; commit any formatting-only diff.

---

## Task 5: On-device verification

**Files:** none (verification)

This is the acceptance gate for hosted mode — the memory win and the GPU-serialization correctness.

- [ ] **Step 1: Build and deploy**

Build the ARM compositor (1a) and `bmc-wasm-host` (with overlays) and deploy via `scripts/nix-cargo-deploy.sh` (it
deploys the compositor and native binaries incl. the host). Set `DEVICE_IP`. Do **not** run the standalone
`validation-overlay` for this test — the point is the host rendering it.

- [ ] **Step 2: Confirm hosted rendering**

Start a widget so the host launches (the host starts on the first widget). Expected:

- The validation overlay (green wash + marker box) appears over the scene, rendered by the **host process** (confirm
  only `bmc-wasm-host` is running it; no separate overlay process).

- It shares the host renderer: there is still exactly one `FemtoVgRenderer`/GL context in the host (no second EGL
  context spun up for the overlay) — confirm via host logs (one "shared wasm host renderer initialized").

- Touch on the marker reaches the overlay (temporary `on_touch` log).

- [ ] **Step 3: GPU-serialization and memory checks (BDK-509 watch)**

  - Run with an animating scene/widget for several minutes: no MMU-fault / scene-freeze. The overlay renders under the
    same `/run/bmc-gpu-render.lock` + GL-fence handoff as widgets (it goes through `acquire_gpu_render_lock` +
    `flush_and_wait_gl`); confirm no fault regression.
  - Memory: hosting the overlay in-process must not add a second renderer's worth of GPU memory. Spot-check host RSS and
    GPU memory against a baseline without the overlay.
  - Interaction with widget rendering: widgets continue to render and cycle normally with the overlay up; no starvation
    or frame stalls beyond the expected serialization.

- [ ] **Step 4: Teardown**

When the overlay client closes (`Closed`) or the host shuts down: overlay disappears, scene repaints clean (no stale
pixels — this also exercises plan-1a's damage-on-unmap), no leaked buffers, host exits cleanly. Record results in the
PR.

---

## Self-review notes

- Spec coverage: implements the hosted entrypoint (`HostedOverlay` + host loop integration) sharing the single
  `FemtoVgRenderer` — the memory consolidation the spec is built around. The host renders overlays one at a time inside
  its single loop, lending the renderer per render call (the spec's single-user guarantee), and routes them through the
  existing GPU render lock + GL-fence handoff (the spec's GPU-serialization constraint for hosted mode), verified on
  device in Task 5 Step 3.
- Honest gaps to resolve against the compiler: the orchestration in Task 2 reuses host-internal helpers
  (`normalize_gl_state`, `blit_staging_to`, `acquire_gpu_render_lock`, `current_ref().fbo`) located in
  `slot.rs`/`host.rs` research; visibility may need widening (some are private to `slot.rs`). Each is flagged at its
  step. If `normalize_gl_state` is slot-private, hoist it next to `render_hosted_overlay` or into `host.rs`.
- The poll-set and timeout wiring (Task 3) mirrors the existing slot bookkeeping; the exact pollfd index bookkeeping
  must follow whatever structure `run_loop` already uses for slots.
- Out of scope: `deck_screen_edge_v1`, the top-edge gesture, neighbor→Dormant, and the swipe panel (spec Step 3/4); the
  real IP/offline overlays (spec Step 2). The validation overlay is removed once Step 2's overlays land, per the spec.
- After 1a+1b+1c land, the framework is proven both standalone (1b) and hosted (1c); Step 2 overlays then implement
  `SystemOverlay` and register in `build_overlays` with no further host changes.

# System overlays — Stage 2 implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the two compositor-signal-free system overlays from Stage 2 of the design — a fullscreen startup IP
overlay and a bottom-right offline indicator — and retire the throwaway validation overlay.

**Architecture:** Both overlays are `SystemOverlay` impls compiled into `bmc-wasm-host` via `build_overlays`. They
declare a desired visibility each `tick`, but cache OS connectivity probes internally so the host's frame loop does not
run `getifaddrs(3)` every pass. The framework maps/unmaps their layer surface accordingly — allocating a buffer on show
and freeing it on hide (the "buffers while hidden" memory rule). Connectivity is shared sync helpers:
`primary_ipv4() -> Option<Ipv4Addr>` and `configured_station_ssid() -> Option<String>`. The offline indicator maps while
IPv4 is absent; the startup IP overlay maps immediately at operational startup, shows the configured station SSID while
waiting, then shows success with the IP or failure after the wait timeout.

**Layer placement (avoids the double "offline" signal):** the startup IP overlay sits on `Layer::Top` (rank 2) and the
offline indicator on `Layer::Bottom` (rank 1); `Layer::Overlay` (rank 3) is reserved for the future screen-edge swipe
panel (Step 3–4). The compositor paints all layer ranks above the scene ordered by `layer_rank`, so the fullscreen,
**opaque** `Top` startup overlay fully occludes the `Bottom` offline chip while both are mapped at boot — no
cross-process coordination needed. Because the boot screen is no longer on `Layer::Overlay`, Stage-1's scene-drag
suppression (`fullscreen_overlay_active`, keyed to `Overlay`) would stop firing for it, so Task 1b broadens that
predicate (renamed `fullscreen_blocker_active`) to any fullscreen layer surface above `Background`. (Offline-on-`Bottom`
rendering above the scene depends on this compositor painting every layer rank over the scene rather than the standard
wlr convention; that is intentional and in-house.)

**Tech Stack:** Rust 2024, `wlr-layer-shell` (client), `bmc-render` femtovg renderer, `get_if_addrs` for sync interface
enumeration, OpenWrt's `uci` CLI for the saved station SSID (no hand-parsing of `/etc/config/wireless`), and the
existing `bmc-system-overlay` framework crate.

---

## Stage-1 alignment (what actually landed vs. the design)

Read this before starting — Stage 1 shipped more than the design's "Step 1" scoped, which shrinks Stage 2.

**Already done in Stage 1 (do not redo):**

- Framework crate `bmc-system-overlay` with `SystemOverlay` trait,
  `LayerConfig`/`InputRegion`/`TickOutcome`/`TouchEvent`, `LayerSurfaceClient`, `OverlayRenderTarget`, `wait_for_gpu`,
  `TreeUi`, `run_standalone`, `HostedOverlay`. (`system-overlays/bmc-system-overlay/src/*`)
- Host integration: `build_overlays`, `render_hosted_overlay`, and the main-loop tick/poll/render/drop wiring for
  overlays. (`bmc-wasm-host/src/overlays.rs`, `src/main_loop.rs`)
- Compositor: `wlr-layer-shell` enabled; layer surfaces composited above the scene with per-pixel alpha; layer-surface
  buffer registry (`LayerEntry` with `buffer`/`buffer_id`/`last_geometry`); **NULL-buffer unmap already releases the
  buffer and invalidates the texture** (`BufferAssignment::Removed` branch in `commit_layer_surface`); full-output
  damage on buffer change; touch hit-test of layer surfaces before widgets honoring the input region; scene-drag
  suppressed and scene-swipe neighbors demoted `Prepared`→`Dormant` under a fullscreen overlay — but keyed to
  `Layer::Overlay`, so Task 1b must generalize it once the boot screen moves to `Layer::Top`.
  (`bmc-openwrt/src/compositor/{layer_surface,state,scene_renderer,egl_compositor}.rs`)
- Throwaway `ValidationOverlay` + standalone `validation-overlay` bin + `layer-shell-test-client`, all wired and
  building.

**Open questions resolved by this plan:**

- *Prepared-vs-Dormant semantics*: already implemented in Stage 1 (`suppress_prepared`); no Stage-2 work.
- *IP overlay dismiss*: a touch-down hides the overlay immediately; otherwise success/failure states time out. The
  implementation is operational-startup only and does not solve initial setup, initial WiFi connect, reconfiguration, AP
  mode, or captive portal flow.

**The one real framework gap Stage 2 must close:** the framework can only *map* (it renders and attaches a buffer); it
has no *unmap/hide* and no *re-show*. Both Stage-2 overlays need dynamic visibility (IP dismisses → unmap forever;
offline toggles with connectivity). Task 1 adds this. The compositor side of hide already exists (Stage 1), so this is
purely client/framework-side.

**Stage-2 connectivity decision:** "online" is defined as *any non-loopback interface holding a non-link-local IPv4
address*. This intentionally means IPv4-presence, not internet reachability and not link-carrier state. Because the
product is WiFi-centric, `primary_ipv4()` prefers WiFi station interfaces — matched by the kernel `wlan*` name prefix,
not a fixed name, because the trailing index is not stable across boots/platforms — before falling back to deterministic
lexicographic interface-name order; it must not depend on raw `getifaddrs(3)` enumeration order. To show what WiFi the
startup overlay is waiting on, the helper reads the configured enabled station SSID via OpenWrt's `uci` CLI
(`uci -q show wireless`), not by hand-parsing `/etc/config/wireless` — `uci` normalizes quoting, comments, and includes,
and is the supported accessor. It does not initiate or repair a connection. We deliberately do **not** pull in
`ii-net-drv` (it drags `tokio` + `wl-nl80211`) or the device-side async `Manager` WiFi accessors (same reason);
`get_if_addrs` is already a workspace dep used for exactly this in `bmc-openwrt/src/unix.rs`, and `uci` is a synchronous
subprocess with no extra dependency. If product wants "offline" to mean no uplink despite a configured/static address,
that is a different signal and a follow-up change, not part of this Stage-2 plan.

---

## File structure

- **Create** `system-overlays/bmc-system-overlay/src/connectivity.rs` — sync `primary_ipv4()` helper (prefers `wlan*`)
  shared by both overlays, plus `configured_station_ssid()` (via `uci`) for the startup overlay's waiting/failure text.
- **Modify** `system-overlays/bmc-system-overlay/src/overlay.rs` — add `visible` to `TickOutcome`; place `fullscreen` on
  `Layer::Top` and add `LayerConfig::bottom_right` on `Layer::Bottom`.
- **Modify** `bmc-openwrt/src/compositor/{layer_surface,state,egl_compositor}.rs` (Task 1b) — broaden + rename
  `is_fullscreen_overlay` → `is_fullscreen_blocker` so scene drag is suppressed under any fullscreen layer surface above
  `Background`.
- **Modify** `system-overlays/bmc-system-overlay/src/surface.rs` — add `attach_null_buffer` (unmap) to
  `LayerSurfaceClient`.
- **Modify** `system-overlays/bmc-system-overlay/src/gpu.rs` — add `OverlayRenderTarget::free_for_hide`, cleaning up
  cached `wl_buffer`s through `LayerSurfaceClient` so the client's slot bookkeeping stays consistent.
- **Modify** `system-overlays/bmc-system-overlay/src/hosted.rs` + `src/standalone.rs` — drive map/unmap from `visible`.
- **Modify** `system-overlays/bmc-system-overlay/src/lib.rs` + `Cargo.toml` — export helper; add `get_if_addrs` dep;
  drop `validation`.
- **Modify** `system-overlays/bmc-system-overlay/src/validation.rs` — temporary compile bridge: keep the throwaway
  validation overlay visible until Task 5 removes it.
- **Create** `system-overlays/bmc-overlay-ip/{Cargo.toml,src/lib.rs,src/main.rs}` — IP overlay crate (lib + standalone
  bin).
- **Create** `system-overlays/bmc-overlay-offline/{Cargo.toml,src/lib.rs,src/main.rs}` — offline overlay crate.
- **Modify** `bmc-wasm-host/src/overlays.rs` — `build_overlays` builds the two real overlays instead of
  `ValidationOverlay`.
- **Modify** root `Cargo.toml` — add the two crates to `members` + `[workspace.dependencies]`; remove
  `validation-overlay`.
- **Delete** `system-overlays/bmc-system-overlay/src/validation.rs` and `system-overlays/validation-overlay/`.

Naming decision (design left crate names open): `bmc-overlay-ip` and `bmc-overlay-offline`. Confirm before Task 3.

---

## Task 1: Framework — dynamic visibility (map / unmap / re-show)

Adds the "buffers while hidden" client path. An overlay's `tick` now reports whether it wants to be on-screen; the
framework maps it (allocate + render) when `visible` flips true and unmaps it (NULL buffer + free buffers) when it flips
false.

**Files:**

- Modify: `system-overlays/bmc-system-overlay/src/overlay.rs`

- Modify: `system-overlays/bmc-system-overlay/src/surface.rs`

- Modify: `system-overlays/bmc-system-overlay/src/gpu.rs`

- Modify: `system-overlays/bmc-system-overlay/src/hosted.rs`

- Modify: `system-overlays/bmc-system-overlay/src/standalone.rs`

- Modify: `system-overlays/bmc-system-overlay/src/validation.rs`

- [ ] **Step 1: Add `visible` to `TickOutcome` and a corner `LayerConfig` constructor**

In `overlay.rs`, extend `TickOutcome`:

```rust
/// Result of an overlay's per-pass background work.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickOutcome {
    /// Whether the overlay wants to be on-screen. When `false` the framework
    /// unmaps the surface (NULL buffer) and frees its export buffers; when it
    /// flips back to `true` the framework reallocates and renders a fresh frame.
    pub visible: bool,
    /// The overlay's content changed and it wants a redraw this pass. Ignored
    /// while `visible` is `false`.
    pub wants_render: bool,
    /// Earliest instant the overlay wants to be ticked again. `None` means
    /// "only on external events".
    pub next_wake: Option<Instant>,
}
```

In the same file add a corner constructor next to `fullscreen`:

```rust
impl LayerConfig {
    /// A small overlay pinned to the bottom-right corner with no input region,
    /// on the `Bottom` layer so a fullscreen `Top`/`Overlay` surface occludes it.
    #[must_use]
    pub fn bottom_right(namespace: impl Into<String>, size: (u32, u32)) -> Self {
        Self {
            layer: Layer::Bottom,
            anchor: Anchor::Bottom | Anchor::Right,
            size,
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: namespace.into(),
            input: InputRegion::None,
        }
    }
}
```

Because `ValidationOverlay` is still present until Task 5, update its `tick` return in
`system-overlays/bmc-system-overlay/src/validation.rs` so Task 1 compiles:

```rust
    fn tick(&mut self, _now: Instant) -> TickOutcome {
        // Render once; a later resize sets surface-dirty separately, so this
        // does not need to keep asking. No periodic wake.
        TickOutcome {
            visible: true,
            wants_render: !self.rendered,
            next_wake: None,
        }
    }
```

- [ ] **Step 2: Add `attach_null_buffer` to `LayerSurfaceClient`**

In `surface.rs`, add a method that unmaps the surface by committing a NULL buffer:

```rust
    /// Unmap the surface: attach a NULL buffer and commit. The compositor
    /// releases the previously-attached buffer and evicts its texture (handled
    /// compositor-side on the `Removed` assignment).
    pub fn attach_null_buffer(&mut self) -> anyhow::Result<()> {
        let surface = self.state.surface.as_ref().context("surface not created")?;
        surface.attach(None, 0, 0);
        surface.commit();
        self.conn
            .flush()
            .map_err(|e| anyhow::anyhow!("wl flush on unmap: {e}"))
    }
```

- [ ] **Step 3: Add `free_for_hide` to `OverlayRenderTarget`**

In `gpu.rs`, add a non-terminal free that leaves the target reusable (unlike `destroy`, which is for shutdown).
`DoubleBufferState::destroy_all` already leaves slots reallocatable via lazy `ensure_current`; we additionally drop the
cached `wl_buffer`s through `LayerSurfaceClient::destroy_minted_wl_buffer` and reset release bookkeeping so a later show
starts clean. Do not call `buffer.destroy()` directly here: the client owns the `buffer_slots`/`released_buffers`
bookkeeping for minted buffers.

```rust
    /// Free the GBM/GL export buffers and cached `wl_buffer`s for a hide, but
    /// keep the target reusable: a later `ensure_current` reallocates lazily.
    /// Distinct from [`Self::destroy`], which is terminal (shutdown only).
    pub fn free_for_hide(
        &mut self,
        egl: &EglContext,
        client: &mut crate::surface::LayerSurfaceClient,
    ) {
        self.buffers.destroy_all(egl);
        for wl_buffer in &mut self.wl_buffers {
            if let Some(buffer) = wl_buffer.take() {
                client.destroy_minted_wl_buffer(buffer);
            }
        }
        self.release = SlotReleaseState::new();
    }
```

- [ ] **Step 4: Drive map/unmap from `visible` in `HostedOverlay`**

In `hosted.rs`, add a `visible` field (default `false`, i.e. starts unmapped) and a `mapped` field, update `tick`, and
gate rendering.

First add these small predicates near `MIN_INTER_FRAME` so the render/hide gate is unit-testable without opening a
Wayland connection:

```rust
#[derive(Debug, Clone, Copy)]
struct RenderGate {
    failed: bool,
    visible: bool,
    mapped: bool,
    wants_render: bool,
    inter_frame_ok: bool,
    client_running: bool,
    target_available: bool,
}

#[must_use]
fn overlay_needs_render(gate: RenderGate) -> bool {
    let wants = gate.wants_render || (gate.visible && !gate.mapped);
    !gate.failed
        && gate.visible
        && wants
        && gate.inter_frame_ok
        && gate.client_running
        && gate.target_available
}

#[must_use]
fn overlay_needs_hide(mapped: bool, visible: bool) -> bool {
    mapped && !visible
}
```

Then replace the `tick`, `needs_render`, `needs_hide`, `hide`, and `mark_rendered` logic:

```rust
    // add to the struct:
    //   visible: bool,
    //   mapped: bool,
    // initialized to false in connect().

    /// Run background work; updates visibility, render-want and next-wake.
    pub fn tick(&mut self, now: Instant) {
        let outcome = self.overlay.tick(now);
        self.visible = outcome.visible;
        if outcome.visible {
            self.wants_render |= outcome.wants_render;
        }
        self.next_wake = outcome.next_wake;
    }

    /// Whether a frame should be rendered+submitted this pass. A first show
    /// (visible but not yet mapped) always renders, even without `wants_render`.
    #[must_use]
    pub fn needs_render(&self, now: Instant) -> bool {
        let inter_frame_ok = self
            .last_render
            .is_none_or(|t| now.duration_since(t) >= MIN_INTER_FRAME);
        overlay_needs_render(RenderGate {
            failed: self.failed,
            visible: self.visible,
            mapped: self.mapped,
            wants_render: self.wants_render,
            inter_frame_ok,
            client_running: self.client.running(),
            target_available: self.target.available(),
        })
    }

    /// Whether the overlay is mapped but no longer wants to be — the host must
    /// unmap and free its buffers this pass.
    #[must_use]
    pub fn needs_hide(&self) -> bool {
        overlay_needs_hide(self.mapped, self.visible)
    }

    /// Unmap the surface and free export buffers. Called by the host when
    /// `needs_hide` is true.
    pub fn hide(&mut self, egl: &EglContext) -> anyhow::Result<()> {
        // Ordering is load-bearing: flush the NULL attach before destroying
        // exported buffers so the compositor observes the unmap first.
        self.client.attach_null_buffer()?;
        self.target.free_for_hide(egl, &mut self.client);
        self.mapped = false;
        self.wants_render = false;
        Ok(())
    }

    /// Mark a render as completed at `now`; the surface is now mapped.
    pub fn mark_rendered(&mut self, now: Instant) {
        self.last_render = Some(now);
        self.wants_render = false;
        self.mapped = true;
    }
```

Do not add special `poll_timeout` handling for hide: the host handles `needs_hide()` in the same pass immediately after
`tick()` sets `visible = false`, so an extra zero-timeout branch would be dead.

- [ ] **Step 5: Call `hide` from the host main loop**

In `bmc-wasm-host/src/main_loop.rs`, in the overlay render loop (the `for overlay in overlays.iter_mut()` block that
calls `overlay.tick(now)`), handle hide before render:

```rust
        for overlay in overlays.iter_mut() {
            overlay.tick(now);
            if overlay.needs_hide() {
                if let Err(e) = overlay.hide(&shared.egl) {
                    tracing::error!("overlay hide error, dropping overlay: {e}");
                    overlay.mark_failed();
                }
                continue;
            }
            if overlay.needs_render(now)
                && let Err(e) =
                    crate::overlays::render_hosted_overlay(overlay, renderer_ptr, shared, now)
            {
                if shared.is_context_lost() {
                    return Err(FatalError::EglContextLost);
                }
                tracing::error!("overlay render error, dropping overlay: {e}");
                overlay.mark_failed();
            }
        }
```

- [ ] **Step 6: Drive map/unmap from `visible` in `run_standalone`**

In `standalone.rs`, track `mapped` and honor `visible`. Replace the body of the `while client.running()` loop's decision
logic:

```rust
        let now = Instant::now();
        let tick = overlay.tick(now);
        if tick.visible {
            if tick.wants_render || !mapped || client.take_needs_render() {
                pending_render = true;
            }
        } else {
            let _ = client.take_needs_render();
            if mapped {
                client.attach_null_buffer()?;
                target.free_for_hide(&egl, &mut client);
                mapped = false;
                pending_render = false;
                last_render = None;
            }
        }

        let inter_frame_remaining = last_render
            .and_then(|t| MIN_INTER_FRAME.checked_sub(now.duration_since(t)))
            .filter(|d| !d.is_zero());

        if pending_render && target.available() && inter_frame_remaining.is_none() {
            render_frame(/* unchanged args */)?;
            pending_render = false;
            mapped = true;
            last_render = Some(now);
        }
```

Declare `let mut mapped = false;` alongside `pending_render`. The timeout computation below is unchanged.

- [ ] **Step 7: Add pure visibility tests for the hosted render gate**

Add these tests at the bottom of `hosted.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn runnable_gate(visible: bool, mapped: bool, wants_render: bool) -> RenderGate {
        RenderGate {
            failed: false,
            visible,
            mapped,
            wants_render,
            inter_frame_ok: true,
            client_running: true,
            target_available: true,
        }
    }

    #[test]
    fn first_show_renders_without_dirty_flag() {
        assert!(overlay_needs_render(runnable_gate(true, false, false)));
    }

    #[test]
    fn hidden_ignores_latched_render_request() {
        assert!(!overlay_needs_render(runnable_gate(false, false, true)));
    }

    #[test]
    fn mapped_but_invisible_needs_hide() {
        assert!(overlay_needs_hide(true, false));
    }

    #[test]
    fn throttled_first_show_waits_for_frame_floor() {
        let mut gate = runnable_gate(true, false, false);
        gate.inter_frame_ok = false;

        assert!(!overlay_needs_render(gate));
    }
}
```

- [ ] **Step 8: `cargo check` + `cargo test` the framework crate**

Run: `rtk nix develop .#fast -c cargo test -p bmc-system-overlay --lib` Expected: PASS, including the hosted visibility
tests and the existing overlay config tests.

- [ ] **Step 9: Commit**

```bash
rtk git add system-overlays/bmc-system-overlay bmc-wasm-host/src/main_loop.rs
rtk git commit -F - <<'EOF'
bmc-system-overlay: bmc-wasm-host: Add show/hide visibility #BDK-416

- add a visible flag to TickOutcome so an overlay can ask to be unmapped
- free export buffers and attach a NULL buffer on hide, reallocate on show
- drive map/unmap from the host loop and the standalone loop
EOF
```

---

## Task 1b: Place the boot overlay on `Top`; suppress scene drag there

The startup overlay must be fullscreen and block scene interaction, but `Layer::Overlay` is reserved for the future
screen-edge swipe panel, so the boot screen goes on `Layer::Top`. Stage-1's scene-drag suppression
(`fullscreen_overlay_active`) is keyed to `Layer::Overlay`; the compositor-level swipe gesture in `on_touch_motion` is
gated *only* by that predicate (the layer input-region hit-test sets touch focus but does not pre-empt the gesture), so
without this change a stray swipe would silently navigate the scene behind the opaque boot screen. The right rule is not
"on the `Top` layer" but "a fullscreen surface covers the scene" — which is true on any layer above `Background`. So
broaden the predicate to exempt only `Background`, and rename it (`is_fullscreen_overlay` → `is_fullscreen_blocker`,
`fullscreen_overlay_active` → `fullscreen_blocker_active`) since it no longer means "the overlay layer". The future edge
panel will not false-trigger it: it is not fullscreen until fully expanded, at which point suppression is correct.

**Files:**

- Modify: `system-overlays/bmc-system-overlay/src/overlay.rs`

- Modify: `bmc-openwrt/src/compositor/layer_surface.rs`

- Modify: `bmc-openwrt/src/compositor/{state,egl_compositor}.rs` (carry the rename through the one caller + its field)

- [ ] **Step 1: Move `fullscreen` to the `Top` layer**

In `overlay.rs`, the existing `fullscreen` constructor currently builds `layer: Layer::Overlay`. Change it to
`layer: Layer::Top` (leave anchors, size, and `InputRegion::Full` unchanged). The `fullscreen_config_anchors_all_edges`
test does not assert the layer, so it still passes.

- [ ] **Step 2: Generalize and rename the predicate**

In `bmc-openwrt/src/compositor/layer_surface.rs`, a fullscreen surface on *any* layer above the background covers the
scene, so suppression must not be tied to a specific layer:

```rust
#[must_use]
pub fn is_fullscreen_blocker(
    layer: Layer,
    geo: Rectangle<i32, Logical>,
    output: Size<i32, Logical>,
) -> bool {
    layer != Layer::Background
        && geo.loc.x <= 0
        && geo.loc.y <= 0
        && geo.size.w >= output.w
        && geo.size.h >= output.h
}
```

Carry the rename through the single caller and its state plumbing: `fullscreen_overlay_active` →
`fullscreen_blocker_active` in `state.rs` (and its call sites in `egl_compositor.rs` — the `on_touch_motion` drag gate
and the render/lifecycle checks), and the `last_fullscreen_overlay_active` field → `last_fullscreen_blocker_active`.

Replace the layer-specific tests with ones that encode the intent: a full-cover surface returns `true` for `Overlay`,
`Top`, and `Bottom`; `false` for `Background`; and `false` for a non-fullscreen (corner) surface.

- [ ] **Step 3: Build + test the compositor crate**

Run: `rtk nix develop .#ci -c cargo test -p bmc-openwrt --lib layer_surface` (compiles the whole crate, so the rename in
`state.rs`/`egl_compositor.rs` is checked too) Expected: PASS, including the full-cover `Top`/`Bottom` cases and the
`Background` exemption.

- [ ] **Step 4: Commit**

```bash
rtk git add system-overlays/bmc-system-overlay/src/overlay.rs bmc-openwrt/src/compositor
rtk git commit -F - <<'EOF'
bmc-system-overlay: bmc: compositor: Place boot overlay on Top layer #BDK-416

- put the fullscreen startup overlay on Top, reserving Overlay for the
  screen-edge swipe panel
- suppress scene drag and demote swipe neighbors under any fullscreen layer
  above the background, not only Overlay
- rename is_fullscreen_overlay to is_fullscreen_blocker accordingly
EOF
```

---

## Task 2: Framework — shared connectivity helper

**Files:**

- Create: `system-overlays/bmc-system-overlay/src/connectivity.rs`

- Modify: `system-overlays/bmc-system-overlay/src/lib.rs`

- Modify: `system-overlays/bmc-system-overlay/Cargo.toml`

- Modify: root `Cargo.toml` (only if `get_if_addrs` is not already in `[workspace.dependencies]` — it is, at 0.5.3)

- [ ] **Step 1: Add the module with its pure tests**

Create `system-overlays/bmc-system-overlay/src/connectivity.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

//! Synchronous, low-cadence connectivity probe shared by OS-driven overlays.
//!
//! "Online" means: at least one non-loopback interface holds a routable IPv4
//! address (link-local 169.254/16 excluded). The device is WiFi-centric and the
//! codebase exposes no separate ethernet carrier probe, so IPv4 presence is the
//! single signal for "neither WiFi nor ethernet connected".
//!
//! The startup IP overlay also needs the saved station SSID for display text.
//! It comes from OpenWrt's `uci` CLI (which normalizes quoting/comments), and is
//! intentionally observational: this helper does not start, retry, repair, or
//! reconfigure WiFi.

use std::net::Ipv4Addr;

use get_if_addrs::{IfAddr, Interface};

/// True if `ip` is usable for connectivity (not loopback, not link-local).
#[must_use]
fn is_routable(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()
}

/// Return the routable IPv4 for one interface if it has one.
#[must_use]
fn interface_ipv4(iface: &Interface) -> Option<Ipv4Addr> {
    match &iface.addr {
        IfAddr::V4(v4) if !iface.is_loopback() && is_routable(v4.ip) => Some(v4.ip),
        IfAddr::V4(_) | IfAddr::V6(_) => None,
    }
}

/// Pick the preferred routable IPv4 from an interface list. Pure, for testing.
///
/// Prefer WiFi station interfaces (kernel `wlan*` names; the trailing index is
/// not stable across boots/platforms, so match the prefix, not a fixed name).
/// Otherwise fall back to lexicographic interface-name order so the result is
/// deterministic and does not depend on raw `getifaddrs(3)` enumeration order.
#[must_use]
fn pick_ipv4(interfaces: &[Interface]) -> Option<Ipv4Addr> {
    let mut candidates: Vec<(&str, Ipv4Addr)> = interfaces
        .iter()
        .filter_map(|iface| interface_ipv4(iface).map(|ip| (iface.name.as_str(), ip)))
        .collect();
    // wlan* first (false sorts before true), then lexicographic within a group.
    candidates.sort_by_key(|(name, _)| (!name.starts_with("wlan"), *name));
    candidates.first().map(|(_, ip)| *ip)
}

/// The device's primary IPv4 address, or `None` when offline. This performs a
/// `getifaddrs(3)` walk; overlays must call it behind their own poll cache, not
/// once per host frame.
#[must_use]
pub fn primary_ipv4() -> Option<Ipv4Addr> {
    let interfaces = get_if_addrs::get_if_addrs().ok()?;
    pick_ipv4(&interfaces)
}

/// First enabled station-mode SSID from `uci show wireless` output. Pure, for
/// testing. The output is one `key=value` line per option, values single-quoted
/// and already comment-free; sections appear as `wireless.<id>=<type>`.
#[must_use]
fn station_ssid_from_uci_show(output: &str) -> Option<String> {
    #[derive(Default)]
    struct Section {
        mode: Option<String>,
        ssid: Option<String>,
        disabled: bool,
    }
    let mut sections: Vec<(String, Section)> = Vec::new();
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim_matches('\'');
        let mut parts = key.split('.');
        if parts.next() != Some("wireless") {
            continue;
        }
        let Some(id) = parts.next() else { continue };
        match parts.next() {
            None if value == "wifi-iface" => sections.push((id.to_owned(), Section::default())),
            None => {}
            Some(option) => {
                let Some((_, section)) = sections.last_mut().filter(|(sid, _)| sid == id) else {
                    continue;
                };
                match option {
                    "mode" => section.mode = Some(value.to_owned()),
                    "ssid" => section.ssid = Some(value.to_owned()),
                    "disabled" => section.disabled = matches!(value, "1" | "true" | "yes" | "on"),
                    _ => {}
                }
            }
        }
    }
    sections
        .into_iter()
        .filter(|(_, s)| s.mode.as_deref() == Some("sta") && !s.disabled)
        .find_map(|(_, s)| s.ssid.filter(|ssid| !ssid.is_empty()))
}

/// Saved station SSID via OpenWrt's `uci` (not by hand-parsing the config file):
/// run `uci -q show wireless` and select the first enabled station section.
/// Synchronous subprocess; safe for the startup overlay's low-cadence `tick`.
/// Observational only — never starts, retries, or reconfigures WiFi.
#[must_use]
pub fn configured_station_ssid() -> Option<String> {
    let output = std::process::Command::new("uci")
        .args(["-q", "show", "wireless"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    station_ssid_from_uci_show(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use get_if_addrs::Ifv4Addr;

    fn v4(name: &str, ip: Ipv4Addr) -> Interface {
        Interface {
            name: name.to_owned(),
            addr: IfAddr::V4(Ifv4Addr {
                ip,
                netmask: Ipv4Addr::new(255, 255, 255, 0),
                broadcast: None,
            }),
        }
    }

    #[test]
    fn prefers_wifi_ipv4_before_ethernet_even_when_ethernet_is_first() {
        let ifaces = vec![
            v4("lo", Ipv4Addr::new(127, 0, 0, 1)),
            v4("eth0", Ipv4Addr::new(192, 168, 1, 50)),
            v4("wlan0", Ipv4Addr::new(10, 0, 0, 5)),
        ];
        assert_eq!(pick_ipv4(&ifaces), Some(Ipv4Addr::new(10, 0, 0, 5)));
    }

    #[test]
    fn falls_back_to_lexicographically_first_routable_interface() {
        let ifaces = vec![
            v4("zz0", Ipv4Addr::new(10, 0, 0, 9)),
            v4("aa0", Ipv4Addr::new(192, 168, 1, 50)),
        ];
        assert_eq!(pick_ipv4(&ifaces), Some(Ipv4Addr::new(192, 168, 1, 50)));
    }

    #[test]
    fn prefers_lowest_wlan_index_among_multiple() {
        let ifaces = vec![
            v4("wlan1", Ipv4Addr::new(10, 0, 0, 7)),
            v4("wlan0", Ipv4Addr::new(10, 0, 0, 5)),
        ];
        assert_eq!(pick_ipv4(&ifaces), Some(Ipv4Addr::new(10, 0, 0, 5)));
    }

    #[test]
    fn none_when_only_loopback_and_link_local() {
        let ifaces = vec![
            v4("lo", Ipv4Addr::new(127, 0, 0, 1)),
            v4("wlan0", Ipv4Addr::new(169, 254, 9, 9)),
        ];
        assert_eq!(pick_ipv4(&ifaces), None);
    }

    #[test]
    fn parses_enabled_station_ssid_from_uci_show() {
        let output = "\
wireless.radio0=wifi-device
wireless.radio0.type='mac80211'
wireless.ap=wifi-iface
wireless.ap.mode='ap'
wireless.ap.ssid='Deck setup'
wireless.sta=wifi-iface
wireless.sta.mode='sta'
wireless.sta.ssid='Office WiFi'
wireless.sta.disabled='0'
";
        assert_eq!(
            station_ssid_from_uci_show(output),
            Some("Office WiFi".to_owned())
        );
    }

    #[test]
    fn skips_disabled_station_in_uci_show() {
        let output = "\
wireless.old=wifi-iface
wireless.old.mode='sta'
wireless.old.disabled='1'
wireless.old.ssid='Old WiFi'
wireless.new=wifi-iface
wireless.new.mode='sta'
wireless.new.ssid='New WiFi'
";
        assert_eq!(
            station_ssid_from_uci_show(output),
            Some("New WiFi".to_owned())
        );
    }

    #[test]
    fn none_when_only_ap_mode_in_uci_show() {
        let output = "\
wireless.ap=wifi-iface
wireless.ap.mode='ap'
wireless.ap.ssid='Deck setup'
";
        assert_eq!(station_ssid_from_uci_show(output), None);
    }
}
```

- [ ] **Step 2: Wire the module only, then run a compile sanity check**

In `lib.rs`, add:

```rust
mod connectivity;
pub use connectivity::{configured_station_ssid, primary_ipv4};
```

Run: `rtk nix develop .#fast -c cargo test -p bmc-system-overlay --lib connectivity` Expected: FAIL to compile with an
unresolved import/use of `get_if_addrs`. This is only a wiring sanity check: it proves the new module is compiled before
the dependency is added. If it fails for any unrelated reason, stop and fix that instead of treating it as a useful red
test.

- [ ] **Step 3: Wire the dependency**

In `system-overlays/bmc-system-overlay/Cargo.toml`, add under `[dependencies]`:

```toml
get_if_addrs.workspace = true
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `rtk nix develop .#fast -c cargo test -p bmc-system-overlay --lib connectivity` Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
rtk git add system-overlays/bmc-system-overlay
rtk git commit -F - <<'EOF'
bmc-system-overlay: Add synchronous connectivity probe #BDK-416

- add primary_ipv4 returning the device IPv4 or None when offline
- prefer wlan* station interfaces before deterministic fallback ordering
- add configured_station_ssid reading the saved SSID via the uci CLI
- skip loopback and link-local addresses; low-cadence getifaddrs walk
EOF
```

---

## Task 3: `bmc-overlay-ip` — startup IP overlay

Fullscreen operational-startup overlay that maps immediately, even before the device has an IP address. It mirrors the
old stable boot behavior from `/home/fbw/doc/work/bmc-main-stable-26.02/bmc/src/display_tasks.rs`: show the configured
station SSID while waiting for an IP, show success with the IP if one appears before the timeout, or show failure if the
timeout expires. It does **not** run the initial-connect/setup flow from `initial_setup.rs`; it only observes the saved
WiFi config and current IP state. A touch-down hides it immediately from any phase and unmaps it forever.

> **Scope decision — design-approved.** This overlay reproduces the legacy boot-status UX from `display_tasks.rs`
> (connecting → SSID → success/failure), which is more than the original one-line "startup IP overlay." Design approved
> the expansion and the spec's Step 2 was updated to match
> (`docs/superpowers/specs/2026-06-07-system-overlays-design.md`). The phase durations below mirror stable: 10 retries ×
> 2 s wait (20 s), 10 s success, 5 s failure.

**Files:**

- Create: `system-overlays/bmc-overlay-ip/Cargo.toml`

- Create: `system-overlays/bmc-overlay-ip/src/lib.rs`

- Create: `system-overlays/bmc-overlay-ip/src/main.rs`

- Modify: root `Cargo.toml`

- [ ] **Step 1: Create the crate manifest**

`system-overlays/bmc-overlay-ip/Cargo.toml`:

```toml
[package]
name = "bmc-overlay-ip"
version = "0.1.0"
edition = "2024"
authors = ["Braiins Systems s.r.o."]
description = "Startup overlay showing the device IP address"

[lib]
path = "src/lib.rs"

[[bin]]
name = "bmc-overlay-ip"
path = "src/main.rs"

[dependencies]
anyhow.workspace = true
bmc-render.workspace = true
bmc-system-overlay.workspace = true

[lints]
workspace = true
```

- [ ] **Step 2: Write the failing test (startup connection state machine)**

`system-overlays/bmc-overlay-ip/src/lib.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

//! Fullscreen operational-startup overlay: show WiFi/IP connection progress,
//! then success or failure, then unmap for the rest of the session.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{
    LayerConfig, SystemOverlay, TickOutcome, TouchEvent, configured_station_ssid, primary_ipv4,
};

/// How long to wait for an IPv4 before showing the connection-failure state.
const WAIT_FOR_IP: Duration = Duration::from_secs(20);
/// How long the success state (connected + IP) stays up before auto-dismiss.
const SUCCESS_VISIBLE_FOR: Duration = Duration::from_secs(10);
/// How long the failure state stays up before auto-dismiss.
const FAILURE_VISIBLE_FOR: Duration = Duration::from_secs(5);
/// Connectivity re-check cadence while waiting for an address.
const POLL: Duration = Duration::from_secs(1);

/// Injected connectivity source so the state machine is unit-testable.
trait Env {
    fn ipv4(&self) -> Option<Ipv4Addr>;
    fn station_ssid(&self) -> Option<String>;
}

struct OsEnv;
impl Env for OsEnv {
    fn ipv4(&self) -> Option<Ipv4Addr> {
        primary_ipv4()
    }

    fn station_ssid(&self) -> Option<String> {
        configured_station_ssid()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Mapped immediately at operational startup; polling for an IPv4.
    Connecting { since: Instant },
    /// IPv4 appeared before timeout; show the last-known IP for a fixed duration.
    Success { since: Instant, ip: Ipv4Addr },
    /// Timeout expired without IPv4; show failure briefly.
    Failed { since: Instant },
    /// Touch/timeout dismissed; unmapped permanently.
    Done,
}

#[must_use]
fn phase_visible(phase: Phase) -> bool {
    !matches!(phase, Phase::Done)
}

/// Pure transition for one tick. Returns the next phase and whether the status
/// text changed so a redraw is warranted.
fn step(phase: Phase, now: Instant, ip: Option<Ipv4Addr>) -> (Phase, bool) {
    match phase {
        Phase::Connecting { since } => {
            if let Some(ip) = ip {
                (Phase::Success { since: now, ip }, true)
            } else if now.duration_since(since) >= WAIT_FOR_IP {
                (Phase::Failed { since: now }, true)
            } else {
                (Phase::Connecting { since }, false)
            }
        }
        Phase::Success { since, ip: shown_ip } => {
            if now.duration_since(since) >= SUCCESS_VISIBLE_FOR {
                (Phase::Done, false)
            } else if let Some(ip) = ip.filter(|ip| *ip != shown_ip) {
                (Phase::Success { since, ip }, true)
            } else {
                // Keep showing the last-known IP through transient DHCP/interface
                // loss. Dismissal is only touch-down or success/failure timeout.
                (Phase::Success { since, ip: shown_ip }, false)
            }
        }
        Phase::Failed { since } => {
            if now.duration_since(since) >= FAILURE_VISIBLE_FOR {
                (Phase::Done, false)
            } else {
                (Phase::Failed { since }, false)
            }
        }
        Phase::Done => (Phase::Done, false),
    }
}

pub struct IpOverlay {
    phase: Phase,
    ip: Option<Ipv4Addr>,
    ssid: Option<String>,
    last_probe: Option<Instant>,
    env: Box<dyn Env>,
}

impl std::fmt::Debug for IpOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpOverlay")
            .field("phase", &self.phase)
            .field("ip", &self.ip)
            .field("ssid", &self.ssid)
            .finish_non_exhaustive()
    }
}

impl Default for IpOverlay {
    fn default() -> Self {
        Self {
            phase: Phase::Connecting { since: Instant::now() },
            ip: None,
            ssid: None,
            last_probe: None,
            env: Box::new(OsEnv),
        }
    }
}

impl IpOverlay {
    fn probe_if_due(&mut self, now: Instant) -> bool {
        if self
            .last_probe
            .is_some_and(|last| now.duration_since(last) < POLL)
        {
            return false;
        }

        self.last_probe = Some(now);
        let next_ip = self.env.ipv4();
        let next_ssid = self.env.station_ssid();
        let changed = self.ip != next_ip || self.ssid != next_ssid;
        self.ip = next_ip;
        self.ssid = next_ssid;
        changed
    }

    #[must_use]
    fn ssid_text(&self, fallback: &str) -> String {
        self.ssid.as_deref().map_or_else(
            || fallback.to_owned(),
            |ssid| format!("WiFi SSID: {ssid}"),
        )
    }
}

impl SystemOverlay for IpOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig::fullscreen("bmc-overlay-ip")
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        if matches!(self.phase, Phase::Done) {
            return TickOutcome {
                visible: false,
                wants_render: false,
                next_wake: None,
            };
        }

        let probe_changed = self.probe_if_due(now);
        let (next, phase_changed) = step(self.phase, now, self.ip);
        self.phase = next;
        let visible = phase_visible(self.phase);
        let next_wake = match self.phase {
            Phase::Connecting { since } => {
                let poll = now + POLL;
                let deadline = since + WAIT_FOR_IP;
                Some(if poll < deadline { poll } else { deadline })
            }
            Phase::Success { since, .. } => Some(since + SUCCESS_VISIBLE_FOR),
            Phase::Failed { since } => Some(since + FAILURE_VISIBLE_FOR),
            Phase::Done => None,
        };
        TickOutcome {
            visible,
            wants_render: visible && (phase_changed || probe_changed),
            next_wake,
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "display dimensions fit comfortably in f32 mantissa"
        )]
        let (w, h) = (size.0 as f32, size.1 as f32);
        // Opaque: this fullscreen Top surface must fully occlude the offline
        // indicator on the Bottom layer while both are mapped at boot.
        r.fill_rect(0.0, 0.0, w, h, Color::from_rgba(0, 0, 0, 255));
        let (title, detail, footer) = match self.phase {
            Phase::Connecting { .. } => (
                "Connecting...",
                self.ssid_text("Waiting for WiFi connection"),
                Some("Waiting for IP address"),
            ),
            Phase::Success { ip, .. } => ("You're connected", format!("http://{ip}/"), None),
            Phase::Failed { .. } => (
                "Problem with connection.",
                self.ssid_text("No WiFi SSID configured"),
                Some("No IP address assigned"),
            ),
            Phase::Done => return,
        };

        draw_centered(r, title, w, h / 2.0 - 52.0, 44.0);
        draw_centered(r, &detail, w, h / 2.0, 32.0);
        if let Some(footer) = footer {
            draw_centered(r, footer, w, h / 2.0 + 44.0, 26.0);
        }
    }

    fn on_touch(&mut self, event: TouchEvent) {
        if matches!(event, TouchEvent::Down { .. }) {
            self.phase = Phase::Done;
        }
    }
}

fn draw_centered(r: &mut dyn Renderer, text: &str, width: f32, y: f32, font: f32) {
    let text_width = r.measure_text(text, font);
    r.draw_text(
        text,
        (width - text_width) / 2.0,
        y,
        font,
        Color::from_rgba(255, 255, 255, 255),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn t0() -> Instant {
        Instant::now()
    }

    struct StaticEnv {
        ip: Option<Ipv4Addr>,
    }

    impl Env for StaticEnv {
        fn ipv4(&self) -> Option<Ipv4Addr> {
            self.ip
        }

        fn station_ssid(&self) -> Option<String> {
            None
        }
    }

    struct CountingEnv {
        calls: Rc<Cell<usize>>,
        ip: Option<Ipv4Addr>,
    }

    impl Env for CountingEnv {
        fn ipv4(&self) -> Option<Ipv4Addr> {
            self.calls.set(self.calls.get() + 1);
            self.ip
        }

        fn station_ssid(&self) -> Option<String> {
            None
        }
    }

    #[test]
    fn connecting_is_visible_without_ip() {
        let start = t0();
        let (next, changed) = step(Phase::Connecting { since: start }, start + POLL, None);

        assert_eq!(next, Phase::Connecting { since: start });
        assert!(phase_visible(next));
        assert!(!changed);
    }

    #[test]
    fn connecting_succeeds_when_ip_appears() {
        let now = t0();
        let ip = Ipv4Addr::new(10, 0, 0, 5);
        let (next, changed) = step(Phase::Connecting { since: now }, now + POLL, Some(ip));

        assert_eq!(next, Phase::Success { since: now + POLL, ip });
        assert!(changed);
    }

    #[test]
    fn connecting_fails_after_ip_timeout() {
        let start = t0();
        let later = start + WAIT_FOR_IP;
        let (next, changed) = step(Phase::Connecting { since: start }, later, None);

        assert_eq!(next, Phase::Failed { since: later });
        assert!(changed);
    }

    #[test]
    fn success_auto_dismisses_after_display_duration() {
        let start = t0();
        let (next, _) = step(
            Phase::Success {
                since: start,
                ip: Ipv4Addr::new(10, 0, 0, 5),
            },
            start + SUCCESS_VISIBLE_FOR,
            Some(Ipv4Addr::new(10, 0, 0, 5)),
        );

        assert_eq!(next, Phase::Done);
    }

    #[test]
    fn success_keeps_last_ip_through_transient_probe_loss() {
        let start = t0();
        let shown_ip = Ipv4Addr::new(10, 0, 0, 5);
        let (next, changed) = step(
            Phase::Success {
                since: start,
                ip: shown_ip,
            },
            start + POLL,
            None,
        );

        assert_eq!(
            next,
            Phase::Success {
                since: start,
                ip: shown_ip,
            }
        );
        assert!(!changed);
    }

    #[test]
    fn tick_reuses_cached_probe_between_poll_intervals() {
        let start = t0();
        let calls = Rc::new(Cell::new(0));
        let mut overlay = IpOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: None,
            last_probe: None,
            env: Box::new(CountingEnv {
                calls: Rc::clone(&calls),
                ip: None,
            }),
        };

        let _ = overlay.tick(start);
        let _ = overlay.tick(start + Duration::from_millis(500));

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn touch_down_hides_immediately() {
        let start = t0();
        let mut overlay = IpOverlay {
            phase: Phase::Connecting { since: start },
            ip: None,
            ssid: None,
            last_probe: None,
            env: Box::new(StaticEnv { ip: None }),
        };

        overlay.on_touch(TouchEvent::Down { id: 0, x: 0.0, y: 0.0 });
        let tick = overlay.tick(start);

        assert_eq!(overlay.phase, Phase::Done);
        assert!(!tick.visible);
        assert_eq!(tick.next_wake, None);
    }
}
```

- [ ] **Step 3: Create the standalone bin**

`system-overlays/bmc-overlay-ip/src/main.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_overlay_ip::IpOverlay;
use bmc_system_overlay::run_standalone;

fn main() -> anyhow::Result<()> {
    run_standalone(Box::new(IpOverlay::default()))
}
```

- [ ] **Step 4: Register the crate**

In root `Cargo.toml`, add to `members`: `"system-overlays/bmc-overlay-ip",` and to `[workspace.dependencies]`:
`bmc-overlay-ip = { path = "system-overlays/bmc-overlay-ip" }`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `rtk nix develop .#fast -c cargo test -p bmc-overlay-ip --lib` Expected: PASS (7 tests).

- [ ] **Step 6: Commit**

```bash
rtk git add system-overlays/bmc-overlay-ip Cargo.toml
rtk git commit -F - <<'EOF'
bmc-overlay-ip: Add startup IP-address overlay #BDK-416

- show fullscreen startup status immediately while waiting for WiFi/IP
- show the saved station SSID, then success IP or failure after timeout
- hide on touch-down or after the success/failure display duration
- cache connectivity probes so host frame rate does not drive getifaddrs
- add a standalone binary entrypoint
EOF
```

---

## Task 4: `bmc-overlay-offline` — offline indicator

Bottom-right indicator, empty input region, mapped only while offline. Visual contract, verified against
`/home/fbw/doc/work/bmc-main-stable-26.02/bmc-display/ui/controls/status-overlay.slint`: the label is exactly `OFFLINE`
(capital case), drawn in the bottom-right corner on a red see-through background.

**Files:**

- Create: `system-overlays/bmc-overlay-offline/Cargo.toml`

- Create: `system-overlays/bmc-overlay-offline/src/lib.rs`

- Create: `system-overlays/bmc-overlay-offline/src/main.rs`

- Modify: root `Cargo.toml`

- [ ] **Step 1: Create the crate manifest**

`system-overlays/bmc-overlay-offline/Cargo.toml`:

```toml
[package]
name = "bmc-overlay-offline"
version = "0.1.0"
edition = "2024"
authors = ["Braiins Systems s.r.o."]
description = "Bottom-right indicator shown while the device is offline"

[lib]
path = "src/lib.rs"

[[bin]]
name = "bmc-overlay-offline"
path = "src/main.rs"

[dependencies]
anyhow.workspace = true
bmc-render.workspace = true
bmc-system-overlay.workspace = true

[lints]
workspace = true
```

- [ ] **Step 2: Write the failing test (visibility tracks connectivity)**

`system-overlays/bmc-overlay-offline/src/lib.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

//! Bottom-right "OFFLINE" indicator. Mapped only while the device has no
//! routable IPv4; unmaps when connectivity returns and remaps if it drops.

use std::time::{Duration, Instant};

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{LayerConfig, SystemOverlay, TickOutcome, primary_ipv4};

/// Indicator size in logical pixels.
const SIZE: (u32, u32) = (200, 56);
/// Legacy display text; keep capital case.
const LABEL: &str = "OFFLINE";
/// Red see-through background.
const BACKGROUND_RGBA: (u8, u8, u8, u8) = (180, 0, 0, 160);
/// Connectivity re-check cadence.
const POLL: Duration = Duration::from_secs(2);

/// Injected connectivity source for testing.
trait Env {
    fn online(&self) -> bool;
}

struct OsEnv;
impl Env for OsEnv {
    fn online(&self) -> bool {
        primary_ipv4().is_some()
    }
}

/// Pure: given current online state and last-rendered visibility, decide the
/// next `(visible, wants_render)`.
fn decide(online: bool, was_visible: bool) -> (bool, bool) {
    let visible = !online;
    let wants_render = visible && !was_visible;
    (visible, wants_render)
}

pub struct OfflineOverlay {
    visible: bool,
    online: bool,
    last_probe: Option<Instant>,
    env: Box<dyn Env>,
}

impl std::fmt::Debug for OfflineOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineOverlay")
            .field("visible", &self.visible)
            .field("online", &self.online)
            .finish_non_exhaustive()
    }
}

impl Default for OfflineOverlay {
    fn default() -> Self {
        Self {
            visible: false,
            online: true,
            last_probe: None,
            env: Box::new(OsEnv),
        }
    }
}

impl OfflineOverlay {
    fn probe_if_due(&mut self, now: Instant) {
        if self
            .last_probe
            .is_some_and(|last| now.duration_since(last) < POLL)
        {
            return;
        }

        self.last_probe = Some(now);
        self.online = self.env.online();
    }
}

impl SystemOverlay for OfflineOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig::bottom_right("bmc-overlay-offline", SIZE)
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        self.probe_if_due(now);
        let (visible, wants_render) = decide(self.online, self.visible);
        self.visible = visible;
        TickOutcome {
            visible,
            wants_render,
            next_wake: Some(now + POLL),
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "indicator dimensions fit comfortably in f32 mantissa"
        )]
        let (w, h) = (size.0 as f32, size.1 as f32);
        let (bg_r, bg_g, bg_b, bg_a) = BACKGROUND_RGBA;
        r.fill_rounded_rect(
            0.0,
            0.0,
            w,
            h,
            12.0,
            Color::from_rgba(bg_r, bg_g, bg_b, bg_a),
        );
        let text = LABEL;
        let font = 28.0;
        let tw = r.measure_text(text, font);
        r.draw_text(
            text,
            (w - tw) / 2.0,
            h / 2.0 + font / 3.0,
            font,
            Color::from_rgba(255, 255, 255, 255),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct CountingEnv {
        calls: Rc<Cell<usize>>,
        online: bool,
    }

    impl Env for CountingEnv {
        fn online(&self) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.online
        }
    }

    #[test]
    fn offline_maps_and_renders_on_transition() {
        assert_eq!(decide(false, false), (true, true));
    }

    #[test]
    fn offline_stays_mapped_without_extra_render() {
        assert_eq!(decide(false, true), (true, false));
    }

    #[test]
    fn online_unmaps() {
        assert_eq!(decide(true, true), (false, false));
    }

    #[test]
    fn constants_match_legacy_offline_indicator() {
        let (red, green, blue, alpha) = BACKGROUND_RGBA;

        assert_eq!(LABEL, "OFFLINE");
        assert!(red > green);
        assert!(red > blue);
        assert!(alpha > 0);
        assert!(alpha < u8::MAX);
    }

    #[test]
    fn tick_reuses_cached_probe_between_poll_intervals() {
        let start = Instant::now();
        let calls = Rc::new(Cell::new(0));
        let mut overlay = OfflineOverlay {
            visible: false,
            online: true,
            last_probe: None,
            env: Box::new(CountingEnv {
                calls: Rc::clone(&calls),
                online: false,
            }),
        };

        let _ = overlay.tick(start);
        let _ = overlay.tick(start + Duration::from_millis(500));

        assert_eq!(calls.get(), 1);
    }
}
```

- [ ] **Step 3: Create the standalone bin**

`system-overlays/bmc-overlay-offline/src/main.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_overlay_offline::OfflineOverlay;
use bmc_system_overlay::run_standalone;

fn main() -> anyhow::Result<()> {
    run_standalone(Box::new(OfflineOverlay::default()))
}
```

- [ ] **Step 4: Register the crate**

In root `Cargo.toml`, add to `members`: `"system-overlays/bmc-overlay-offline",` and to `[workspace.dependencies]`:
`bmc-overlay-offline = { path = "system-overlays/bmc-overlay-offline" }`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `rtk nix develop .#fast -c cargo test -p bmc-overlay-offline --lib` Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
rtk git add system-overlays/bmc-overlay-offline Cargo.toml
rtk git commit -F - <<'EOF'
bmc-overlay-offline: Add offline indicator overlay #BDK-416

- show a bottom-right OFFLINE indicator while no routable IPv4 is present
- use a red see-through background matching the legacy indicator
- empty input region so corner touches fall through
- unmap on reconnect and remap if connectivity drops
- cache connectivity probes so host frame rate does not drive getifaddrs
EOF
```

---

## Task 5: Wire overlays into the host; retire the validation overlay

**Files:**

- Modify: `bmc-wasm-host/src/overlays.rs`

- Modify: `bmc-wasm-host/Cargo.toml`

- Modify: `system-overlays/bmc-system-overlay/src/lib.rs`

- Delete: `system-overlays/bmc-system-overlay/src/validation.rs`

- Delete: `system-overlays/validation-overlay/`

- Modify: root `Cargo.toml`

- [ ] **Step 1: Build the real overlays in `build_overlays`**

Replace `build_overlays` in `bmc-wasm-host/src/overlays.rs`:

```rust
use bmc_overlay_ip::IpOverlay;
use bmc_overlay_offline::OfflineOverlay;
use bmc_system_overlay::{HostedOverlay, SystemOverlay};

/// Build the compiled-in system overlays. Each opens its own Wayland
/// connection and allocates buffers from `egl`. A failure to start one overlay
/// is logged and skipped, never fatal to the host.
pub fn build_overlays(egl: &EglContext) -> Vec<HostedOverlay> {
    // Stacking is by layer rank, not build order: the offline indicator is on
    // the Bottom layer and the startup IP overlay on Top, so the fullscreen IP
    // overlay occludes the offline chip regardless of the order built here.
    let factories: Vec<(&str, fn() -> Box<dyn SystemOverlay>)> = vec![
        ("offline", || Box::new(OfflineOverlay::default())),
        ("ip", || Box::new(IpOverlay::default())),
    ];
    let mut overlays = Vec::new();
    for (name, make) in factories {
        match HostedOverlay::connect(make(), egl) {
            Ok(o) => overlays.push(o),
            Err(e) => tracing::error!("failed to start {name} overlay: {e}"),
        }
    }
    overlays
}
```

(Remove the `ValidationOverlay` import; keep `render_hosted_overlay` unchanged.)

- [ ] **Step 2: Add host dependencies**

In `bmc-wasm-host/Cargo.toml`, under `[dependencies]` add:

```toml
bmc-overlay-ip = { workspace = true }
bmc-overlay-offline = { workspace = true }
```

- [ ] **Step 3: Remove the validation overlay from the framework**

- Delete `system-overlays/bmc-system-overlay/src/validation.rs`.

- In `lib.rs` remove `mod validation;` and `pub use validation::ValidationOverlay;`.

- Delete the directory `system-overlays/validation-overlay/`.

- In root `Cargo.toml`, remove `"system-overlays/validation-overlay",` from `members`.

Decision: keep `system-overlays/layer-shell-test-client` — it is a generic protocol smoke-test, not the throwaway
overlay the design said to remove. Confirm with the author; remove too if unwanted.

- [ ] **Step 4: Build the whole workspace touched by this change**

Run: `rtk nix develop .#fast -c cargo build -p bmc-wasm-host` Expected: PASS, no reference to `ValidationOverlay` or
`validation-overlay` remains.

- [ ] **Step 5: Clippy + fmt the touched crates**

Run:
`rtk nix develop .#fast -c cargo clippy -p bmc-system-overlay -p bmc-overlay-ip -p bmc-overlay-offline -p bmc-wasm-host --tests -- -D warnings`
Then: `rtk nix fmt` Expected: clean.

- [ ] **Step 6: Commit**

```bash
rtk git add bmc-wasm-host system-overlays Cargo.toml
rtk git commit -F - <<'EOF'
bmc-wasm-host: Host the IP and offline overlays #BDK-416

- build the IP and offline overlays in place of the validation overlay
- remove the throwaway validation overlay and its standalone binary
EOF
```

---

## Task 6: On-device verification

No automated GPU tests exist; these are manual, on the Braiins Deck, per the design's verification list.

- [ ] **Step 1: Deploy and watch logs**

Build the ARMv7 host and deploy per `docs/nix-device-scripts.md` (`scripts/nix-cargo-deploy.sh` does not deploy wasm
widgets, but the overlays are native and ship with `bmc-wasm-host`; confirm the deploy path includes the host binary).
Tail the host log for `Layer surface ready` lines for `bmc-overlay-ip` and `bmc-overlay-offline`.

- [ ] **Step 2: IP overlay appears immediately, then succeeds or fails**

Boot with saved WiFi reachable. Confirm: the fullscreen IP overlay appears immediately, before the IP is assigned; while
waiting it shows `Connecting...` and the saved station SSID when one is configured; when the IP appears within ~20 s it
changes to `You're connected` and `http://<ip>/`, then auto-dismisses after ~10 s. Touch down during connecting,
success, or failure hides it in the same interaction pass. After dismiss, the scene behind repaints with no stale pixels
(full-output damage path), and the host log shows no repeated IP-overlay renders afterward (it is unmapped + freed).

Verify the *content*, not just that it mapped (NEW-4 guard against a silent fallback): the SSID shown matches
`uci -q get wireless.@wifi-iface[…].ssid` for the enabled station section, and on a unit with more than one interface up
the shown IP is the `wlan*` address, not an ethernet/other one. Also confirm the boot screen fully occludes the offline
chip — while `Connecting...` is up there is **no** `OFFLINE` indicator bleeding through in the bottom-right corner
(opaque `Top` over `Bottom`).

- [ ] **Step 3: IP overlay failure path, then offline indicator tracking**

Boot or force a saved-WiFi/no-IP case without entering initial setup or reconfiguration. Confirm: the fullscreen IP
overlay stays visible while waiting, then changes to `Problem with connection.` after ~20 s, shows the saved station
SSID when one is configured, and unmaps after ~5 s unless touched earlier. Once the fullscreen overlay is gone, the
bottom-right `OFFLINE` indicator remains mapped while no routable IPv4 is present. Confirm a touch in its corner falls
through to the scene (empty input region). Reconnect: the indicator unmaps and its corner repaints the scene. Disconnect
again: it remaps (re-show path exercised).

- [ ] **Step 4: No MMU-fault / fence regression**

Under the BDK-509 conditions (a widget scene animating while an overlay maps/unmaps repeatedly), confirm no scene-freeze
MMU faults — overlay buffers ride the host GL-fence handoff (`flush_and_wait_gl` in `render_hosted_overlay`) and the
compositor alpha-blend waits on the fence. If faults appear, capture the log and stop; do not paper over with a second
lock.

- [ ] **Step 5: Record results**

Note pass/fail per step in the MR description. Reveal-latency measurement is deferred to Step 4 (swipe panel); not in
scope here.

---

## Self-review notes

- **Spec coverage:** offline indicator (Task 4), startup IP (Task 3), direct OS reads (Task 2), empty vs full input
  regions (set per overlay via `LayerConfig`), NULL-buffer free-on-hide (Task 1 client side; compositor side already in
  Stage 1), hide-repaints-vacated-region (Stage 1 `mark_full_output_damage`; verified in Task 6 Steps 2–3), GPU fence on
  device (Task 6 Step 4), validation-overlay removal (Task 5). The `deck_screen_edge_v1` protocol, top-edge gesture, and
  swipe panel are Steps 3–4, intentionally out of scope.
- **IP startup scope — design-approved:** the connecting/success/failure + SSID behavior reproduces stable boot UX
  beyond the original one-line "startup IP overlay"; design approved it and the spec's Step 2 was updated to match. It
  does not implement the old `initial_setup.rs` connect/retry/reconfiguration flow.
- **IP dismiss:** touch-down hides immediately; success auto-dismisses after 10 s; failure auto-dismisses after 5 s. A
  transient IP loss after success does not dismiss the overlay; it keeps showing the last-known IP until touch or
  timeout.
- **Layer placement:** offline indicator on `Layer::Bottom`, startup overlay on `Layer::Top` (opaque, so it occludes the
  chip), `Layer::Overlay` reserved for the Step-3/4 screen-edge panel. Task 1b broadens + renames the suppression
  predicate (`is_fullscreen_blocker`) to fire for any fullscreen layer surface above `Background`, so the fullscreen
  `Top` boot screen still suppresses scene drag — without it a stray swipe navigates the scene behind it
  (`on_touch_motion` gates drag only on that predicate, not on touch focus).
- **Open question — crate names:** `bmc-overlay-ip`, `bmc-overlay-offline`. Confirm before Task 3.
- **Connectivity dependency:** uses `get_if_addrs` (already vendored) plus a synchronous `uci -q show wireless`
  subprocess for the SSID (no hand-parsed config file), not `ii-net-drv` or the async device-side `Manager` accessors
  (avoids `tokio`/`wl-nl80211`). The helpers are synchronous, but each overlay caches probes behind its `POLL` interval
  because `bmc-wasm-host` calls `tick()` every loop pass. Stage 2 deliberately defines online as routable IPv4 presence;
  if a future overlay needs WiFi-association, carrier, active connection attempts, or uplink reachability detail,
  revisit the signal instead of stretching this helper.
- **IPv4 selection:** WiFi station interfaces are preferred by the `wlan*` name prefix (the index is not fixed across
  boots/platforms) before deterministic lexicographic fallback. Do not rely on raw `getifaddrs(3)` enumeration order for
  user-visible IP text, and do not hardcode a specific `wlanN`.
- **Hide ordering:** flush the NULL-buffer commit before destroying exported buffers. That order is intentional and must
  not be "tidied" without checking compositor buffer lifetime.
- **Rendering:** direct `Renderer` draw calls (matching the retired validation overlay), not `TreeUi`. This deviates
  from the design's "declarative tree by default" because the visuals are a single centered line; `TreeUi` stays
  available for richer Step-4 overlays. Surface for review.

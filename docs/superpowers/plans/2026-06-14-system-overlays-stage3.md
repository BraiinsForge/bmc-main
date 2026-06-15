# System overlays — Stage 3 implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reveal a `wlr-layer-shell` overlay from the top screen edge via a vendored `deck_screen_edge_v1` protocol,
with generic client-side support in `bmc-system-overlay` and a throwaway top-edge verification overlay that proves the
whole loop end-to-end (the Stage-3 analogue of Stage-1's `ValidationOverlay`).

**Architecture:** A vendored-and-renamed Wayland protocol (`deck_screen_edge_v1`, forked from `kde-screen-edge-v1`) adds
two interfaces: a manager *global* factory and a per-surface *auto-hide edge* object carrying `activate`/`deactivate`
requests and new `revealed`/`hidden` events. The compositor hand-writes the server `Dispatch`, recognizes a top-edge
downward gesture (gated on a top hot-zone), and on trigger emits `revealed` to the armed edge — marking that session
revealed so scene-drag is suppressed and scene-swipe neighbors demote to `Dormant` (reusing Stage-2's
`fullscreen_blocker`-keyed machinery, now OR'd with `any_screen_edge_revealed`). The framework binds the manager, arms
the edge at startup (hidden + zero-buffer), allocates-and-renders on `revealed`, and frees + re-arms on hide through the
existing Stage-2 `HostedOverlay` map/unmap gates; the host render path prepares a remapped layer surface before drawing
the next frame.

**Tech Stack:** Rust 2024, `wayland-scanner` server/client codegen (vendored XML, mirroring `bmc-widget-protocol`),
Smithay `wlr-layer-shell`, `wayland-client`, the existing `bmc-system-overlay` framework + `bmc-render` femtovg
renderer.

---

## Dependency on Stage 2 (read first)

**This plan assumes the Stage-2 plan (`docs/superpowers/plans/2026-06-08-system-overlays-stage2.md`) is fully merged,
and it is:** the `bmc-overlay-device-info` and `bmc-overlay-offline` crates exist, `validation-overlay/` is gone, and
`build_overlays` (`bmc-wasm-host/src/overlays.rs`) builds `OfflineOverlay` + `DeviceInfoOverlay` from a factory `Vec`.
Stage 3 builds on these **post-Stage-2** shapes; before starting, verify each of these against the tree and stop if any
is missing — it would mean the tree is not at the expected Stage-2 baseline and the diffs will not apply cleanly:

- `is_fullscreen_overlay` → **`is_fullscreen_blocker`** (`layer_surface.rs`), exempts only `Background`.
- `fullscreen_overlay_active()` → **`fullscreen_blocker_active()`** (`state.rs`).
- `last_fullscreen_overlay_active` → **`last_fullscreen_blocker_active`** (`egl_compositor.rs` `AppState`). Task 3
  renames this latch again to `last_neighbors_suppressed`.
- `LayerConfig::fullscreen` lives on **`Layer::Top`**; **`Layer::Overlay` is reserved** for exactly this screen-edge
  panel — the verification overlay uses `Layer::Overlay`.
- `TickOutcome` has a **`visible`** field; `HostedOverlay` has `visible`/`mapped`/`needs_hide`/`hide`/`mark_rendered`
  and the pure `RenderGate`/`overlay_needs_render`/`overlay_needs_hide` helpers; `run_standalone` tracks `mapped`.
- **The actual Stage-2 overlay crates are `bmc-overlay-device-info` (the startup overlay; the Stage-2 plan's tentative
  `bmc-overlay-ip` name was resolved to this) and `bmc-overlay-offline`** — do not assume the `-ip` name anywhere.
- **`build_overlays` (`bmc-wasm-host/src/overlays.rs`) builds overlays from a factory `Vec<OverlayFactory>` where
  `OverlayFactory = (&'static str, fn() -> Box<dyn SystemOverlay>)`** — currently `("offline", ...)` and
  `("device-info", ...)`. Task 7 Step 4 adds one more entry to that `Vec`.
- The Stage-1 `ValidationOverlay` + `validation-overlay/` crate are already **deleted**; `layer-shell-test-client` is
  kept.

---

## File structure

- **Create** `deck-screen-edge-v1/Cargo.toml`, `deck-screen-edge-v1/protocol/deck-screen-edge-v1.xml`,
  `deck-screen-edge-v1/src/lib.rs` — the vendored protocol crate, at the workspace root beside `bmc-widget-protocol`
  (decided in the design: a shared protocol crate, not under the overlay folder).
- **Create** `bmc-openwrt/src/compositor/screen_edge.rs` — server session state, pure flag helpers (+ tests), and the
  `GlobalDispatch`/`Dispatch` impls on `CompositorState`.
- **Modify** `bmc-openwrt/src/compositor.rs` — `mod screen_edge;`.
- **Modify** `bmc-openwrt/src/compositor/state.rs` — `screen_edge_sessions` field + init, the `any_screen_edge_revealed`
  / `neighbors_suppressed` / `trigger_screen_edge` / `surface_has_layer_role` methods, session cleanup on
  `layer_destroyed`, and the `super::screen_edge::create_global` global-advertise call (no `delegate_*` macros — the
  `Dispatch` impls are written directly on `CompositorState`; see Task 2 Step 3).
- **Modify** `bmc-openwrt/src/compositor/egl_compositor.rs` — rename the suppression latch, OR in
  `any_screen_edge_revealed` at the suppression sites, and wire the top-edge gesture into the touch handlers +
  `AppState`.
- **Modify** `bmc-openwrt/src/compositor/touch_gesture.rs` — fold top-edge reveal detection into the existing
  `GestureState` state machine (top, + tests).
- **Modify** `system-overlays/bmc-system-overlay/src/overlay.rs` — add the `ScreenEdge` enum and the `screen_edge` /
  `on_reveal` trait hooks.
- **Modify** `system-overlays/bmc-system-overlay/src/surface.rs` — bind the manager global, create the edge object,
  `activate`/re-arm, and dispatch `revealed`/`hidden`.
- **Modify** `system-overlays/bmc-system-overlay/src/hosted.rs` + `src/standalone.rs` — drive map/unmap from the
  reveal/hide events through the Stage-2 gates.
- **Create** `system-overlays/bmc-system-overlay/src/screen_edge_validation.rs` — throwaway
  `ScreenEdgeValidationOverlay`.
- **Modify** `system-overlays/bmc-system-overlay/src/lib.rs` + `Cargo.toml` — module + exports + protocol dep.
- **Create** `system-overlays/screen-edge-validation-overlay/{Cargo.toml,src/main.rs}` — standalone bin.
- **Modify** `bmc-wasm-host/src/overlays.rs` — add the throwaway overlay to `build_overlays` (no `Cargo.toml` change:
  `bmc-system-overlay` is already a host dependency).
- **Modify** root `Cargo.toml` — register `deck-screen-edge-v1` and `screen-edge-validation-overlay`.

Naming (design left these open, user confirmed): protocol crate `deck-screen-edge-v1` (Rust crate
`deck_screen_edge_v1`), overlay `ScreenEdgeValidationOverlay` + bin `screen-edge-validation-overlay`.

---

## Protocol contract (decided)

The fork inverts the upstream "visible-by-default" model to our zero-buffer-while-hidden rule:

- `deck_screen_edge_manager_v1` — the registry global; factory only: `get_auto_hide_screen_edge(id, border, surface)`.
  Raises `invalid_border` for an out-of-range border, `invalid_role` if `surface` is not a layer surface, and
  `already_constructed` if `surface` already has a screen edge.
- `deck_auto_hide_screen_edge_v1` — one per layer surface. Requests `activate` / `deactivate`; events `revealed` /
  `hidden`.
- **`activate()`** = "go hidden + armed": the compositor arms the edge, clears `revealed`, and emits **`hidden`**. The
  client commits a NULL buffer and frees its DMA-BUFs.
- **top-edge gesture** → the compositor emits **`revealed`** to the first armed top edge, sets it `revealed`, and spends
  the arming. The client allocates, renders, and commits a buffer.
- **Hiding always goes through `activate()`→`hidden`; showing always through `revealed`.** Dismiss (tap/timeout) = the
  client calls `activate()` again. `deactivate()` (disarm + emit `revealed`) is implemented for completeness; the
  verification overlay does not use it.

---

## Task 1: Vendored `deck-screen-edge-v1` protocol crate

Mirror `bmc-widget-protocol`'s `protocol/*.xml` + `lib.rs` (`server`/`client` modules generated by `wayland_scanner`),
no `types.rs`. **One deliberate improvement over `bmc-widget-protocol`:** add a `build.rs` that emits
`cargo:rerun-if-changed` for the XML. `bmc-widget-protocol` has none, so editing its XML does *not* reliably trigger a
rebuild — the `generate_*_code!` proc-macros read the file at expansion time, but cargo does not track that read as a
dependency. The `build.rs` makes XML edits rebuild the crate.

**Files:**

- Create: `deck-screen-edge-v1/Cargo.toml`

- Create: `deck-screen-edge-v1/protocol/deck-screen-edge-v1.xml`

- Create: `deck-screen-edge-v1/build.rs`

- Create: `deck-screen-edge-v1/src/lib.rs`

- Modify: root `Cargo.toml`

- [ ] **Step 1: Write the protocol XML**

`deck-screen-edge-v1/protocol/deck-screen-edge-v1.xml`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<protocol name="deck_screen_edge_v1">
  <copyright>
    Copyright (C) 2026  Braiins Systems s.r.o.

    Forked from kde-screen-edge-v1 (SPDX-FileCopyrightText: 2023 Vlad Zahorodnii,
    SPDX-License-Identifier: MIT-CMU). Renamed and re-contracted: the surface is
    hidden by default and holds no buffer while hidden; revealed/hidden events
    drive allocation and release.
  </copyright>

  <interface name="deck_screen_edge_manager_v1" version="1">
    <description summary="screen edge manager">
      Associates an auto-hide reveal action with a screen edge for a layer
      surface. This is a Deck desktop-shell implementation detail; regular
      clients must not use it.
    </description>

    <enum name="error">
      <entry name="invalid_border" value="0" summary="the specified border value is invalid"/>
      <entry name="invalid_role" value="1" summary="the surface has invalid role"/>
      <entry name="already_constructed" value="2" summary="the surface already has a screen edge"/>
    </enum>

    <request name="destroy" type="destructor">
      <description summary="destroy the screen edge manager">
        Destroy the manager. Does not destroy objects created with it.
      </description>
    </request>

    <enum name="border">
      <description summary="screen border">
        Only the top edge is defined: a top reveal is a downward vertical gesture,
        orthogonal to the horizontal scene swipe. Left/right edges would be
        horizontal gestures that conflict with scene navigation (which can begin
        anywhere, including at a screen edge), and no Stage-3 overlay needs the
        bottom edge, so all are omitted. The value 1 is fixed so a bottom edge
        (value 2) can be added non-breaking when a spec'd overlay requires it.
      </description>
      <entry name="top" value="1" summary="top screen edge"/>
    </enum>

    <request name="get_auto_hide_screen_edge">
      <description summary="create an auto hide edge">
        Create an auto-hide screen edge for the layer surface. Placement stays
        owned by the layer surface; this only associates the reveal trigger.

        invalid_border is raised for an out-of-range border; invalid_role is
        raised unless the surface has the layer_surface role.
      </description>
      <arg name="id" type="new_id" interface="deck_auto_hide_screen_edge_v1"/>
      <arg name="border" type="uint" enum="border"/>
      <arg name="surface" type="object" interface="wl_surface"/>
    </request>
  </interface>

  <interface name="deck_auto_hide_screen_edge_v1" version="1">
    <description summary="auto hide screen edge">
      Hides a layer surface and reveals it on the edge gesture. Unlike upstream,
      the surface is hidden by default and holds no buffer while hidden.
    </description>

    <request name="destroy" type="destructor">
      <description summary="destroy the auto hide screen edge object"/>
    </request>

    <request name="deactivate">
      <description summary="disarm and show">
        Disarm the edge and request the surface be shown. The compositor emits
        revealed.
      </description>
    </request>

    <request name="activate">
      <description summary="arm and hide">
        Arm the edge and request the surface be hidden. The compositor emits
        hidden; the client must commit a NULL buffer and free its buffers. A
        spent edge (already triggered) must be re-armed with this request.
      </description>
    </request>

    <event name="revealed">
      <description summary="the edge was triggered; show the surface">
        The edge gesture fired (or deactivate was called). The client should
        allocate, render, and attach a buffer. The arming is now spent.
      </description>
    </event>

    <event name="hidden">
      <description summary="the surface should be hidden">
        The surface should be hidden. The client should attach a NULL buffer and
        free its DMA-BUFs.
      </description>
    </event>
  </interface>
</protocol>
```

- [ ] **Step 2: Write the crate manifest and build script**

`deck-screen-edge-v1/Cargo.toml` (no `build = "build.rs"` line: cargo auto-detects a `build.rs` at the crate root):

```toml
[package]
name = "deck-screen-edge-v1"
version = "0.1.0"
edition = "2024"
authors = ["Braiins Systems s.r.o."]
description = "Vendored deck_screen_edge_v1 Wayland protocol (forked from kde-screen-edge-v1)"

[dependencies]
wayland-backend.workspace = true
wayland-client.workspace = true
wayland-scanner.workspace = true
wayland-server.workspace = true

[lints]
workspace = true
```

`deck-screen-edge-v1/build.rs` — register the XML as a build dependency so a change to it rebuilds the crate (the
`generate_*_code!` proc-macros read the file at expansion time, which cargo does not otherwise track):

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

fn main() {
    println!("cargo:rerun-if-changed=protocol/deck-screen-edge-v1.xml");
}
```

- [ ] **Step 3: Write `lib.rs`**

`deck-screen-edge-v1/src/lib.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

//! Vendored `deck_screen_edge_v1` Wayland protocol, forked and renamed from
//! `kde-screen-edge-v1`. The surface is hidden by default and holds no buffer
//! while hidden; the added `revealed`/`hidden` events drive allocation and
//! release. The compositor hand-writes the server `Dispatch`; overlays bind the
//! client side through `bmc-system-overlay`.

/// Server-side protocol bindings (for the compositor).
pub mod server {
    #![allow(
        unused_qualifications,
        clippy::all,
        clippy::pedantic,
        missing_debug_implementations
    )]

    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/deck-screen-edge-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("./protocol/deck-screen-edge-v1.xml");
}

/// Client-side protocol bindings (for overlays).
pub mod client {
    #![allow(
        unused_qualifications,
        clippy::all,
        clippy::pedantic,
        missing_debug_implementations
    )]

    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/deck-screen-edge-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("./protocol/deck-screen-edge-v1.xml");
}
```

- [ ] **Step 4: Register the crate in the workspace**

In root `Cargo.toml`, add to `members`: `"deck-screen-edge-v1",` and to `[workspace.dependencies]`:
`deck-screen-edge-v1 = { path = "deck-screen-edge-v1" }`.

- [ ] **Step 5: Build to verify codegen accepts the XML**

Run: `rtk nix develop .#fast -c cargo build -p deck-screen-edge-v1` Expected: PASS — both `server` and `client` modules
compile (the XML is well-formed and references only built-in `wl_surface`).

- [ ] **Step 6: Commit**

```bash
rtk git add deck-screen-edge-v1 Cargo.toml
rtk git commit -F - <<'EOF'
deck-screen-edge-v1: Vendor screen-edge protocol #BDK-416

- fork kde-screen-edge-v1, rename interfaces to deck_screen_edge_v1
- invert to hidden-by-default with no buffer while hidden
- add revealed and hidden events driving allocation and release
- generate server and client bindings, mirroring bmc-widget-protocol
EOF
```

---

## Task 2: Compositor — screen-edge session state and Dispatch

Hand-write the server `Dispatch` directly on `CompositorState` (no generic-over-`D` handler trait: every consumer is in
`bmc-openwrt`, so the indirection the `deck_widget` crate split needs does not apply here). The per-surface flag logic
is factored into pure functions so it is unit-testable without constructing Wayland resources.

**Files:**

- Create: `bmc-openwrt/src/compositor/screen_edge.rs`

- Modify: `bmc-openwrt/src/compositor.rs`

- Modify: `bmc-openwrt/src/compositor/state.rs`

- [ ] **Step 1: Write the session module with pure helpers and their tests**

`bmc-openwrt/src/compositor/screen_edge.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

//! Server state and dispatch for the vendored `deck_screen_edge_v1` protocol.
//!
//! One [`ScreenEdgeSession`] per layer surface that armed an edge. The pure flag
//! transition [`EdgeFlags::try_trigger`] is what the touch-motion path calls, so
//! the production logic is exactly what the tests exercise — no Wayland resources
//! needed. The `Dispatch` impls emit `revealed`/`hidden` on the matching
//! resource.

use deck_screen_edge_v1::server::deck_screen_edge_manager_v1::{
    self, Border, DeckScreenEdgeManagerV1, Error,
};
use deck_screen_edge_v1::server::deck_auto_hide_screen_edge_v1::{
    self, DeckAutoHideScreenEdgeV1,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};

use super::state::CompositorState;

/// Per-surface arm/reveal flags. Split from the Wayland resource so the
/// transition logic is pure and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeFlags {
    pub border: Border,
    /// Armed by `activate`, spent by a trigger.
    pub armed: bool,
    /// True between a trigger (`revealed`) and the next `activate` (`hidden`).
    pub revealed: bool,
}

/// One auto-hide screen edge tied to a layer surface.
#[derive(Debug)]
pub struct ScreenEdgeSession {
    pub resource: DeckAutoHideScreenEdgeV1,
    pub surface: WlSurface,
    pub flags: EdgeFlags,
}

/// UserData for an auto-hide edge resource: the associated layer surface.
#[derive(Debug, Clone)]
pub struct ScreenEdgeUserData {
    pub surface: WlSurface,
}

impl EdgeFlags {
    /// Spend the arming for a trigger on `border`: if this edge is armed on that
    /// border, clear `armed`, set `revealed`, and return `true`; otherwise leave
    /// the flags untouched and return `false`.
    /// `CompositorState::trigger_screen_edge` calls this as its `position`
    /// predicate over the sessions, and the tests below exercise it directly, so
    /// the tested logic is the production logic. (The `border` match is currently
    /// vacuous with only `Border::Top`, but is kept so a future bottom edge needs
    /// no logic change.)
    pub fn try_trigger(&mut self, border: Border) -> bool {
        if self.border == border && self.armed {
            self.armed = false;
            self.revealed = true;
            true
        } else {
            false
        }
    }
}

impl GlobalDispatch<DeckScreenEdgeManagerV1, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckScreenEdgeManagerV1>,
        (): &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<DeckScreenEdgeManagerV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &DeckScreenEdgeManagerV1,
        request: deck_screen_edge_manager_v1::Request,
        (): &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            deck_screen_edge_manager_v1::Request::GetAutoHideScreenEdge {
                id,
                border,
                surface,
            } => {
                let border = match border {
                    WEnum::Value(b) => b,
                    WEnum::Unknown(v) => {
                        manager.post_error(Error::InvalidBorder, format!("invalid border {v}"));
                        return;
                    }
                };
                if !state.surface_has_layer_role(&surface) {
                    manager.post_error(Error::InvalidRole, "surface has no layer_surface role");
                    return;
                }
                if state
                    .screen_edge_sessions
                    .iter()
                    .any(|s| s.surface == surface)
                {
                    manager.post_error(
                        Error::AlreadyConstructed,
                        "surface already has a screen edge",
                    );
                    return;
                }
                let resource = data_init.init(
                    id,
                    ScreenEdgeUserData {
                        surface: surface.clone(),
                    },
                );
                state.screen_edge_sessions.push(ScreenEdgeSession {
                    resource,
                    surface,
                    flags: EdgeFlags {
                        border,
                        armed: false,
                        revealed: false,
                    },
                });
            }
            deck_screen_edge_manager_v1::Request::Destroy => {}
            other => tracing::warn!("Unknown screen-edge manager request: {other:?}"),
        }
    }
}

impl Dispatch<DeckAutoHideScreenEdgeV1, ScreenEdgeUserData> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &DeckAutoHideScreenEdgeV1,
        request: deck_auto_hide_screen_edge_v1::Request,
        _data: &ScreenEdgeUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let Some(session) = state
            .screen_edge_sessions
            .iter_mut()
            .find(|s| s.resource == *resource)
        else {
            return;
        };
        match request {
            deck_auto_hide_screen_edge_v1::Request::Activate => {
                session.flags.armed = true;
                session.flags.revealed = false;
                resource.hidden();
            }
            deck_auto_hide_screen_edge_v1::Request::Deactivate => {
                // Disarm and show explicitly (per the protocol contract). The
                // `session` borrow ends at the flag write above, so the
                // `state` reborrow below is legal under NLL.
                session.flags.armed = false;
                session.flags.revealed = true;
                resource.revealed();
                state.mark_full_output_damage();
            }
            deck_auto_hide_screen_edge_v1::Request::Destroy => {
                state.screen_edge_sessions.retain(|s| s.resource != *resource);
            }
            other => tracing::warn!("Unknown screen-edge request: {other:?}"),
        }
    }
}

/// Advertise the `deck_screen_edge_manager_v1` global.
pub fn create_global(display: &DisplayHandle) {
    display.create_global::<CompositorState, DeckScreenEdgeManagerV1, ()>(1, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(border: Border, armed: bool, revealed: bool) -> EdgeFlags {
        EdgeFlags {
            border,
            armed,
            revealed,
        }
    }

    #[test]
    fn try_trigger_spends_armed_edge_on_matching_border() {
        let mut f = flag(Border::Top, true, false);
        assert!(f.try_trigger(Border::Top));
        assert_eq!(f, flag(Border::Top, false, true));
    }

    #[test]
    fn try_trigger_ignores_unarmed_edge() {
        let mut f = flag(Border::Top, false, false);
        assert!(!f.try_trigger(Border::Top));
        assert_eq!(f, flag(Border::Top, false, false));
    }
}
```

- [ ] **Step 2: Register the module**

In `bmc-openwrt/src/compositor.rs`, add `mod screen_edge;` alongside the other `mod` lines (keep alphabetical order if
the file is ordered).

- [ ] **Step 3: Add session storage, methods, and wiring to `state.rs`**

In `state.rs`, add the field to `CompositorState` (next to `layer_surfaces`, line ~144):

```rust
    pub screen_edge_sessions: Vec<crate::compositor::screen_edge::ScreenEdgeSession>,
```

Initialize it in the `Self { ... }` literal (next to `layer_surfaces: Vec::new(),`, line ~354):

```rust
            screen_edge_sessions: Vec::new(),
```

Advertise the global right after the `deck_widget` one (after line 321 `super::protocol::create_global::<Self>(...)`):

```rust
        super::screen_edge::create_global(&display_handle);
```

Add these methods inside the same `impl CompositorState` block that holds `fullscreen_blocker_active` (after that
method, ~line 744):

```rust
    /// True if `surface` is a tracked layer surface (has the layer_surface
    /// role). Used to reject `get_auto_hide_screen_edge` on non-layer surfaces.
    #[must_use]
    pub fn surface_has_layer_role(&self, surface: &WlSurface) -> bool {
        self.layer_surfaces
            .iter()
            .any(|e| e.surface.wl_surface() == surface)
    }

    /// True while any screen-edge overlay is revealed. OR'd with
    /// `fullscreen_blocker_active` to suppress scene drag and demote neighbors.
    #[must_use]
    pub fn any_screen_edge_revealed(&self) -> bool {
        self.screen_edge_sessions
            .iter()
            .any(|s| s.flags.revealed)
    }

    /// Whether scene interaction must be suppressed and scene-swipe neighbors
    /// demoted to `Dormant`: either a fullscreen blocker is up or a screen-edge
    /// overlay is revealed.
    #[must_use]
    pub fn neighbors_suppressed(&self) -> bool {
        self.fullscreen_blocker_active() || self.any_screen_edge_revealed()
    }

    /// Trigger the first armed session on `border`: spend its arming, mark it
    /// revealed, and emit `revealed`. Returns `true` if an edge was triggered.
    /// `EdgeFlags::try_trigger` mutates only the matched session, so using it as
    /// the `position` predicate spends exactly the first armed edge of `border`;
    /// `position` consumes the iterator before the index access, so the later
    /// `mark_full_output_damage` reborrow is legal.
    pub fn trigger_screen_edge(&mut self, border: Border) -> bool {
        let Some(idx) = self
            .screen_edge_sessions
            .iter_mut()
            .position(|s| s.flags.try_trigger(border))
        else {
            return false;
        };
        self.screen_edge_sessions[idx].resource.revealed();
        self.mark_full_output_damage();
        true
    }
```

Add the `Border` re-export the method uses: at the top of `screen_edge.rs` `Border` is already imported; in `state.rs`
the method imports it locally via the `use` line shown. (`WlSurface` is already imported in `state.rs`.)

In `layer_destroyed` (line ~1130), drop any sessions for the destroyed surface so a revealed flag cannot outlive the
surface. Add at the end of the `if let Some(pos) = ...` body, after `self.mark_full_output_damage();`:

```rust
            self.screen_edge_sessions
                .retain(|s| s.surface != *surface.wl_surface());
```

**No `delegate_*` macros are needed.** Unlike `deck_widget` — whose `Dispatch` impls live on the separate
`DeckWidgetProtocolState` and are forwarded to `CompositorState` by `wl::delegate_dispatch!` (lines 1276–1284) — the
screen-edge `GlobalDispatch`/`Dispatch` impls in Task 2 Step 1 are written *directly on `CompositorState`*. They are
therefore already in scope as `CompositorState`'s own impls; adding `delegate_*` macros would generate a second,
conflicting impl. The only wiring is the `super::screen_edge::create_global(&display_handle)` call added above.

`state.rs` needs exactly one new protocol `use` — `trigger_screen_edge` takes a `Border` parameter — so add it near the
top with the other imports:

```rust
use deck_screen_edge_v1::server::deck_screen_edge_manager_v1::Border;
```

No other protocol imports are needed (method calls like `resource.revealed()` resolve through the field type).

- [ ] **Step 4: Add the `deck-screen-edge-v1` dependency to `bmc-openwrt`**

In `bmc-openwrt/Cargo.toml`, under `[dependencies]`:

```toml
deck-screen-edge-v1 = { workspace = true }
```

- [ ] **Step 5: Build + test the compositor crate**

Run: `rtk nix develop .#ci -c cargo test -p bmc-openwrt --lib screen_edge` Expected: PASS — the three pure flag tests,
and the crate compiles (so the `Dispatch` impls and `state.rs` wiring are type-checked).

- [ ] **Step 6: Commit**

```bash
rtk git add bmc-openwrt
rtk git commit -F - <<'EOF'
bmc: compositor: Dispatch deck_screen_edge_v1 sessions #BDK-416

- track one screen-edge session per layer surface
- handle activate/deactivate, emitting hidden/revealed
- reject get_auto_hide_screen_edge on non-layer surfaces
- add trigger_screen_edge and any_screen_edge_revealed
EOF
```

---

## Task 3: Compositor — reveal-driven scene-drag suppression and neighbor demotion

Stage 2 centralizes neighbor demotion + scene-drag suppression on `fullscreen_blocker_active()`. A revealed
*non-fullscreen* panel would slip past that, so swap those gates to `neighbors_suppressed()` and re-key the per-loop
latch off it, so a screen-edge reveal/hide flips the latch and drives the demote/restore on its own (a reveal happens
during Wayland/touch dispatch, not on a scene command).

**Files:**

- Modify: `bmc-openwrt/src/compositor/egl_compositor.rs`

- [ ] **Step 1: Re-key the suppression latch**

In `AppState` (line ~888), rename the field and its doc:

```rust
    /// Last observed value of [`CompositorState::neighbors_suppressed`]. A layer
    /// map/unmap or screen-edge reveal happens during dispatch, not on a scene
    /// command, so lifecycle would not re-emit on its own. Comparing against
    /// this each loop iteration lets us re-emit when the predicate flips,
    /// demoting scene-swipe neighbors to `Dormant` when an overlay maps or a
    /// screen edge reveals, and restoring them when it goes away.
    last_neighbors_suppressed: bool,
```

Update both initializers: line ~423 (`last_fullscreen_overlay_active: false,` → already renamed by Stage 2 to
`last_fullscreen_blocker_active: false,`) and the test initializer at line ~2247. Both become:

```rust
            last_neighbors_suppressed: false,
```

- [ ] **Step 2: Drive the latch off `neighbors_suppressed`**

At the per-loop check (lines ~633–637), replace the body:

```rust
            let suppressed = app_state.compositor.neighbors_suppressed();
            if suppressed != app_state.last_neighbors_suppressed {
                app_state.last_neighbors_suppressed = suppressed;
                emit_lifecycle_transitions(&mut app_state);
                release_dormant_widget_buffers(&mut app_state);
            }
```

- [ ] **Step 3: Swap the suppression-site predicates**

Replace every `state.compositor.fullscreen_blocker_active()` guard that immediately precedes a `suppress_prepared(...)`
call with `state.compositor.neighbors_suppressed()`. There are four such sites (post-Stage-2 line numbers approximate):

- in `emit_lifecycle_transitions` (line ~1482),
- in `release_dormant_widget_buffers` (line ~1514),
- the release-peek at line ~944,
- the render/lifecycle peek at line ~1873.

Each is the same edit:

```rust
        if state.compositor.neighbors_suppressed() {
            crate::compositor::layer_surface::suppress_prepared(&mut next);
        }
```

(Use the local variable name present at each site — `next` or `lifecycle_states`.)

- [ ] **Step 4: Swap the scene-drag gate**

In `on_touch_motion`, the drag-activation gate (line ~1271) currently reads
`!self.compositor.fullscreen_blocker_active()`. Change it to:

```rust
            && !self.compositor.neighbors_suppressed()
```

so a revealed screen-edge panel also blocks the scene swipe behind it.

- [ ] **Step 5: Build + test the compositor crate**

Run: `rtk nix develop .#ci -c cargo test -p bmc-openwrt --lib` Expected: PASS — existing lifecycle/suppression tests
still pass under the renamed latch and the OR'd predicate (a screen-edge session list that is empty makes
`neighbors_suppressed() == fullscreen_blocker_active()`, so behavior is unchanged when no edge is revealed).

- [ ] **Step 6: Commit**

```bash
rtk git add bmc-openwrt/src/compositor/egl_compositor.rs
rtk git commit -F - <<'EOF'
bmc: compositor: Suppress neighbors while an edge is revealed #BDK-416

- demote scene-swipe neighbors to Dormant while a screen edge is revealed
- suppress scene drag for a revealed non-fullscreen panel too
- re-key the suppression latch off neighbors_suppressed
EOF
```

---

## Task 4: Compositor — top-edge reveal inside `GestureState`

Fold the top-edge reveal into the existing `GestureState` state machine instead of adding a parallel recognizer in
`AppState`. Edge swipe state belongs next to scene-drag state so one pure gesture machine owns the exclusivity rules.
Stage 3 must reveal on the activation motion so the compositor can cancel the forwarded `wl_touch` sequence immediately
and own the rest of the touch.

**Files:**

- Modify: `bmc-openwrt/src/compositor/touch_gesture.rs`

- Modify: `bmc-openwrt/src/compositor/egl_compositor.rs`

- [ ] **Step 1: Add top-edge motion activation to `touch_gesture.rs`**

In `touch_gesture.rs`, add these constants near the existing drag/tap constants:

```rust
/// Fraction of the screen height at the top edge that counts as the hot zone
/// for an edge-reveal gesture. A touch-down must start within this band.
pub const EDGE_HOT_ZONE_FRACTION: f64 = 0.20;

/// Downward movement (logical pixels) required before a top-edge reveal
/// activates.
pub const EDGE_ACTIVATION_DY: f64 = 40.0;

/// Maximum horizontal deviation (logical pixels) allowed during a top-edge
/// reveal. A downward edge gesture may drift horizontally while still being a
/// reveal.
pub const EDGE_MAX_X_DEVIATION: f64 = 150.0;
```

Add the motion-activation enum near `TouchGesture`:

```rust
/// Gesture ownership transition detected during motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionActivation {
    /// No compositor-owned gesture activated on this motion sample.
    None,
    /// Horizontal scene drag crossed its activation threshold.
    SceneDrag,
    /// Downward top-edge reveal crossed its activation threshold.
    TopEdgeReveal,
}
```

Extend `GestureConfig` with the top-edge parameters and the logical screen height:

```rust
    /// Fraction of the logical screen height used as the top edge hot zone.
    pub edge_hot_zone_fraction: f64,
    /// Downward movement (logical pixels) required to trigger a top-edge reveal.
    pub edge_activation_dy: f64,
    /// Maximum horizontal deviation (logical pixels) allowed during the reveal.
    pub edge_max_x_deviation: f64,
    /// Logical screen height. `0.0` disables edge reveal detection, which keeps
    /// `GestureState::default()` usable in tests that do not care about edges.
    pub screen_height: f64,
```

Update `GestureConfig::default()`:

```rust
            edge_hot_zone_fraction: EDGE_HOT_ZONE_FRACTION,
            edge_activation_dy: EDGE_ACTIVATION_DY,
            edge_max_x_deviation: EDGE_MAX_X_DEVIATION,
            screen_height: 0.0,
```

Make `GestureState::with_config` available outside tests (remove the current `#[cfg(test)]` on it), then add edge state
to `GestureState`:

```rust
    top_edge: TopEdgeState,
```

Update `on_down` and add the helper:

```rust
    pub fn on_down(&mut self, location: Point<f64, Logical>, time_ms: u32) {
        self.active = true;
        self.start = location;
        self.current = location;
        self.start_time_ms = time_ms;
        self.drag_active = false;
        self.top_edge = if self.top_edge_contains(location.y) {
            TopEdgeState::Candidate
        } else {
            TopEdgeState::Ineligible
        };
        self.velocity_samples.clear();
        self.velocity_samples.push_back((location.x, time_ms));
    }

    fn top_edge_contains(&self, y: f64) -> bool {
        self.config.screen_height > 0.0
            && y <= self.config.screen_height * self.config.edge_hot_zone_fraction
    }
```

Replace `on_motion` and `update_drag_activation` with a single motion-activation path:

```rust
    /// Update the current touch position. Returns the compositor-owned gesture
    /// that activated on this sample, if any.
    pub fn on_motion(
        &mut self,
        location: Point<f64, Logical>,
        time_ms: u32,
    ) -> MotionActivation {
        if !self.active {
            return MotionActivation::None;
        }
        self.current = location;
        self.push_velocity_sample(location.x, time_ms);
        self.update_motion_activation()
    }

    fn update_motion_activation(&mut self) -> MotionActivation {
        if self.drag_active || matches!(self.top_edge, TopEdgeState::Active) {
            return MotionActivation::None;
        }

        let dx = (self.current.x - self.start.x).abs();
        let dy_signed = self.current.y - self.start.y;

        // Check the edge reveal before scene drag. A downward edge swipe may
        // drift horizontally past the scene-drag dead zone; the vertical
        // activation threshold decides that sample belongs to reveal, and the
        // active guard latches ownership.
        if matches!(self.top_edge, TopEdgeState::Candidate)
            && dy_signed >= self.config.edge_activation_dy
            && dx <= self.config.edge_max_x_deviation
        {
            self.top_edge = TopEdgeState::Active;
            tracing::debug!(
                "Top-edge reveal gesture activated: dy={dy_signed:.1}, dx={dx:.1}"
            );
            return MotionActivation::TopEdgeReveal;
        }

        if dx > self.config.drag_dead_zone && dy_signed.abs() <= self.config.drag_max_y_deviation {
            self.drag_active = true;
            tracing::debug!("Drag activated: dx={:.1}", dx);
            return MotionActivation::SceneDrag;
        }

        MotionActivation::None
    }
```

Update `on_up` and `on_cancel` so edge state is cleared with the rest of the touch state:

```rust
    pub fn on_up(&mut self, time_ms: u32) -> Option<TouchGesture> {
        if !self.active {
            return None;
        }
        self.active = false;
        let gesture = self.classify(time_ms);
        self.drag_active = false;
        self.top_edge = TopEdgeState::Ineligible;
        gesture
    }

    pub fn on_cancel(&mut self) {
        self.active = false;
        self.drag_active = false;
        self.top_edge = TopEdgeState::Ineligible;
    }
```

Add tests inside the existing `#[cfg(test)] mod tests` block. Extend the `use super::{...}` line to include
`MotionActivation`.

```rust
    fn edge_aware() -> GestureState {
        GestureState::with_config(GestureConfig {
            screen_height: 480.0,
            ..GestureConfig::default()
        })
    }

    #[test]
    fn top_edge_reveal_activates_on_motion_from_hot_zone() {
        let mut g = edge_aware();
        g.on_down(p(640.0, 10.0), 0); // y=10 is within 20% of 480 (96px)
        assert_eq!(g.on_motion(p(640.0, 30.0), 10), MotionActivation::None);
        assert_eq!(
            g.on_motion(p(645.0, 60.0), 20),
            MotionActivation::TopEdgeReveal
        );
    }

    #[test]
    fn top_edge_reveal_ignores_touch_started_in_the_middle() {
        let mut g = edge_aware();
        g.on_down(p(640.0, 240.0), 0); // not in the top hot zone
        assert_eq!(g.on_motion(p(640.0, 320.0), 10), MotionActivation::None);
    }

    #[test]
    fn diagonal_downward_swipe_from_top_band_reveals_edge() {
        let mut g = edge_aware();
        g.on_down(p(100.0, 80.0), 0);
        assert_eq!(
            g.on_motion(p(200.0, 130.0), 10),
            MotionActivation::TopEdgeReveal,
            "edge swipes may drift horizontally while moving down"
        );
    }

    #[test]
    fn horizontal_drag_from_top_band_still_navigates_scenes() {
        let mut g = edge_aware();
        g.on_down(p(100.0, 10.0), 0);
        assert_eq!(
            g.on_motion(p(300.0, 25.0), 10),
            MotionActivation::SceneDrag,
            "horizontal motion in the top band is scene drag, not edge reveal"
        );
    }

    #[test]
    fn top_edge_reveal_activates_only_once_per_sequence() {
        let mut g = edge_aware();
        g.on_down(p(640.0, 10.0), 0);
        assert_eq!(
            g.on_motion(p(640.0, 60.0), 10),
            MotionActivation::TopEdgeReveal
        );
        assert_eq!(
            g.on_motion(p(640.0, 120.0), 20),
            MotionActivation::None,
            "no second activation"
        );
    }

    #[test]
    fn top_edge_reveal_prevents_later_scene_drag() {
        let mut g = edge_aware();
        g.on_down(p(640.0, 10.0), 0);
        assert_eq!(
            g.on_motion(p(640.0, 60.0), 10),
            MotionActivation::TopEdgeReveal
        );
        assert_eq!(
            g.on_motion(p(760.0, 80.0), 20),
            MotionActivation::None,
            "once the edge reveal owns the gesture, it cannot promote to drag"
        );
    }
```

- [ ] **Step 2: Update existing gesture tests for `MotionActivation`**

`GestureState::on_motion` no longer returns `bool`; update the existing tests in `touch_gesture.rs`:

- Replace every existing `GestureState::new()` test setup with `GestureState::default()` so Step 4 can delete the
  private constructor after the production callers are switched.
- Replace no-op motion assertions like `assert!(!g.on_motion(...));` with
  `assert_eq!(g.on_motion(...), MotionActivation::None);`.
- Replace drag activation assertions like `assert!(g.on_motion(...), "drag should activate...");` with
  `assert_eq!(g.on_motion(...), MotionActivation::SceneDrag, "drag should activate...");`.

The motion-assertion sites in the current file are the five boolean assertions at lines 261, 276, 278, 289, and 349.
Statement-style `g.on_motion(...)` calls in release/velocity tests compile unchanged, except for replacing their
constructor with `GestureState::default()`. Do not change `TouchGesture::DragEnd`/`Tap` release expectations except for
their intermediate `on_motion` assertions.

- [ ] **Step 3: Run the gesture tests**

Run: `rtk nix develop .#ci -c cargo test -p bmc-openwrt --lib touch_gesture` Expected: PASS — the new top-edge
`GestureState` tests plus the updated horizontal-drag/tap tests.

- [ ] **Step 4: Configure `GestureState` and add the edge-owned flag to `AppState`**

In `egl_compositor.rs`, extend the existing `touch_gesture` import inside the top-level `use super::{ ... }` block:

```rust
    touch_gesture::{GestureConfig, GestureState, MotionActivation, TouchGesture},
```

In `AppState` (near `scene_drag_active`, line ~831), add:

```rust
    /// `true` once the current sequence has been claimed by an edge reveal;
    /// subsequent motion/up are owned by the reveal (no scene navigation, no
    /// forwarding) until the touch lifts.
    edge_reveal_active: bool,
```

Initialize `gesture` with the logical height in the real `AppState { ... }` literal (line ~403):

```rust
            gesture: GestureState::with_config(GestureConfig {
                screen_height: f64::from(logical_height),
                ..GestureConfig::default()
            }),
```

Initialize the flag in the same literal:

```rust
            edge_reveal_active: false,
```

In the test `AppState { ... }` literal (line ~2230), use the test display height:

```rust
            gesture: GestureState::with_config(GestureConfig {
                screen_height: 1280.0,
                ..GestureConfig::default()
            }),
            edge_reveal_active: false,
```

After both `AppState` literals use `GestureState::with_config(...)`, delete `GestureState::new()` instead of leaving it
as a private wrapper around `Default`; the plain-state tests now use `GestureState::default()`, and `GestureState` is
not a public API outside the private compositor module.

- [ ] **Step 5: Reset edge ownership on touch-down**

In `on_touch_down`, after `self.gesture.on_down(location, time);` (line ~1230), add:

```rust
        self.edge_reveal_active = false;
```

- [ ] **Step 6: Arbitrate to edge reveal or scene drag in touch-motion**

In `on_touch_motion`, replace only `let drag_activated = self.gesture.on_motion(location, time);` with this block,
leaving the existing `if drag_activated && ...` scene-drag arbitration immediately below it:

```rust
        let activation = self.gesture.on_motion(location, time);

        if !self.edge_reveal_active && matches!(activation, MotionActivation::TopEdgeReveal) {
            use deck_screen_edge_v1::server::deck_screen_edge_manager_v1::Border;
            if self.compositor.trigger_screen_edge(Border::Top) {
                // Edge reveal claims the sequence: cancel the wl_touch the
                // focused surface is seeing and stop here. No finger-tracking —
                // the compositor already revealed the panel via the event.
                self.edge_reveal_active = true;
                let touch_handle = self.compositor.touch_handle.clone();
                touch_handle.cancel(&mut self.compositor);
                self.touch_frame_dirty = true;
                return;
            } else {
                tracing::info!("top-edge reveal gesture activated, but no armed edge consumed it");
            }
        }
        if self.edge_reveal_active {
            return;
        }

        let drag_activated = matches!(activation, MotionActivation::SceneDrag);
```

`trigger_screen_edge(border)` only returns `true` when an armed edge of that border exists, so an edge-zone swipe with
no panel armed falls through without compositor cancellation and the focused surface keeps the forwarded touch. The
gesture state has still latched `TopEdgeState::Active`, so that same touch will not later promote to `SceneDrag`; this
is intentional, because a vertical reveal attempt should not be reinterpreted mid-sequence as scene navigation.

- [ ] **Step 7: Finalize the edge-owned sequence on up/cancel**

In `on_touch_up`, immediately after `let gesture_result = self.gesture.on_up(time);` (line ~1332) and before the
`scene_drag_active` branch, add the block below. Placing it *after* `on_up` (not between `gesture_slot = None;` and
`on_up`) keeps the gesture state finalized on this path and mirrors the `on_touch_cancel` insertion, which runs after
`gesture.on_cancel()`:

```rust
        if self.edge_reveal_active {
            self.edge_reveal_active = false;
            if touch_sequence_finished {
                self.reset_automatic_waiting(Instant::now());
            }
            return;
        }
```

In `on_touch_cancel`, after `self.gesture.on_cancel();` (line ~1380), add:

```rust
        self.edge_reveal_active = false;
```

- [ ] **Step 8: Build + test the compositor crate**

Run: `rtk nix develop .#ci -c cargo test -p bmc-openwrt --lib` Expected: PASS — the gesture wiring compiles and all
existing touch/lifecycle tests still pass.

- [ ] **Step 9: Commit**

```bash
rtk git add bmc-openwrt/src/compositor/touch_gesture.rs bmc-openwrt/src/compositor/egl_compositor.rs
rtk git commit -F - <<'EOF'
bmc: compositor: Recognize the top edge-reveal gesture #BDK-416

- fold top-edge reveal detection into GestureState
- trigger the armed edge on the swipe, cancelling the forwarded touch
  sequence (forward-then-cancel, no finger-tracking)
- leave the focused surface's touch sequence untouched when no edge is armed
EOF
```

---

## Task 5: Framework — client screen-edge support and trait hook

Add the opt-in trait surface and the `LayerSurfaceClient` plumbing: bind the manager global, create the edge object, arm
it, dispatch `revealed`/`hidden`, and expose drains for the host/standalone loops.

**Files:**

- Modify: `system-overlays/bmc-system-overlay/src/overlay.rs`

- Modify: `system-overlays/bmc-system-overlay/src/surface.rs`

- Modify: `system-overlays/bmc-system-overlay/src/lib.rs`

- Modify: `system-overlays/bmc-system-overlay/Cargo.toml`

- [ ] **Step 1: Add the `ScreenEdge` enum and trait hooks**

In `overlay.rs`, add the enum (after `InputRegion`, ~line 25):

```rust
/// A screen edge an overlay can arm for swipe-reveal. Opt in via
/// [`SystemOverlay::screen_edge`].
///
/// Only the top edge is offered: a top reveal is a downward gesture, orthogonal
/// to the horizontal scene swipe. Bottom and left/right are unsupported —
/// left/right would be horizontal gestures that conflict with scene navigation
/// (which can begin anywhere, including at a screen edge), and no Stage-3 overlay
/// needs the bottom edge. Kept as an enum so a future edge needs no API change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenEdge {
    Top,
}
```

Extend the `SystemOverlay` trait (after `on_touch`, line ~137):

```rust
    /// Opt in to screen-edge reveal. `None` (default) means a normal overlay
    /// whose visibility is driven by [`TickOutcome::visible`]. `Some(edge)` arms
    /// that edge at startup: the surface stays hidden (no buffer) until the
    /// compositor reveals it on the edge gesture, and re-arms on hide.
    fn screen_edge(&self) -> Option<ScreenEdge> {
        None
    }

    /// Called once each time the compositor reveals the overlay's armed edge,
    /// before the first frame of that reveal. Use it to reset per-reveal state.
    fn on_reveal(&mut self) {}
```

- [ ] **Step 2: Add the screen-edge dependency**

In `system-overlays/bmc-system-overlay/Cargo.toml`, under `[dependencies]`:

```toml
deck-screen-edge-v1 = { workspace = true }
```

- [ ] **Step 3: Thread the manager + edge object through `surface.rs`**

In `surface.rs`, extend the imports (after the `wayland_protocols_wlr` line, ~line 21):

```rust
use deck_screen_edge_v1::client::deck_screen_edge_manager_v1::{self, DeckScreenEdgeManagerV1};
use deck_screen_edge_v1::client::deck_auto_hide_screen_edge_v1::{self, DeckAutoHideScreenEdgeV1};

use crate::overlay::ScreenEdge;
```

Add fields to `State` (after `layer_surface: ...`, line ~45):

```rust
    screen_edge_manager: Option<DeckScreenEdgeManagerV1>,
    screen_edge: Option<DeckAutoHideScreenEdgeV1>,
    /// Set true on a `revealed` event; drained by the framework to map+render.
    pending_reveal: bool,
    /// Set true on a `hidden` event; drained by the framework to unmap.
    pending_hidden: bool,
```

Initialize them in `State::default` (after `layer_surface: None,`, line ~73):

```rust
            screen_edge_manager: None,
            screen_edge: None,
            pending_reveal: false,
            pending_hidden: false,
```

Bind the manager in the registry `match` (in the `Dispatch<wl_registry...>` impl, add an arm before the `_ => {}`, line
~344):

```rust
                "deck_screen_edge_manager_v1" => {
                    let manager = registry
                        .bind::<DeckScreenEdgeManagerV1, _, _>(name, version.min(1), qh, ());
                    state.screen_edge_manager = Some(manager);
                }
```

Add the `Dispatch` impls for the two protocol objects (next to the other `Dispatch` impls, e.g. after the
`zwlr_layer_surface_v1` impl, ~line 380):

```rust
impl Dispatch<DeckScreenEdgeManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &DeckScreenEdgeManagerV1,
        _: deck_screen_edge_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<DeckAutoHideScreenEdgeV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &DeckAutoHideScreenEdgeV1,
        event: deck_auto_hide_screen_edge_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            deck_auto_hide_screen_edge_v1::Event::Revealed => {
                state.pending_reveal = true;
                state.pending_hidden = false;
            }
            deck_auto_hide_screen_edge_v1::Event::Hidden => {
                state.pending_hidden = true;
            }
            other => tracing::debug!(?other, "unhandled screen-edge event"),
        }
    }
}
```

Add the public methods on `LayerSurfaceClient` (next to `attach_null_buffer`, ~line 231):

```rust
    /// Create the auto-hide screen edge for this layer surface and arm it
    /// (hidden). The manager global must be present.
    pub fn create_screen_edge(&mut self, edge: ScreenEdge) -> anyhow::Result<()> {
        let qh = self.queue.handle();
        let manager = self
            .state
            .screen_edge_manager
            .clone()
            .context("deck_screen_edge_manager_v1 missing")?;
        let surface = self.state.surface.as_ref().context("surface not created")?;
        let border = match edge {
            ScreenEdge::Top => deck_screen_edge_manager_v1::Border::Top,
        };
        let edge = manager.get_auto_hide_screen_edge(border, surface, &qh, ());
        edge.activate();
        self.state.screen_edge = Some(edge);
        self.flush()
    }

    /// Re-arm the screen edge (go back to hidden + armed). Used on dismiss.
    pub fn rearm_screen_edge(&mut self) -> anyhow::Result<()> {
        let edge = self.state.screen_edge.as_ref().context("no screen edge")?;
        edge.activate();
        self.flush()
    }

    /// Drain a pending `revealed` event (compositor triggered the edge).
    pub fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.state.pending_reveal)
    }

    /// Drain a pending `hidden` event (compositor asked to hide).
    pub fn take_hidden(&mut self) -> bool {
        std::mem::take(&mut self.state.pending_hidden)
    }
```

- [ ] **Step 4: Export `ScreenEdge`**

In `lib.rs`, extend the overlay re-export (line 15):

```rust
pub use overlay::{InputRegion, LayerConfig, ScreenEdge, SystemOverlay, TickOutcome, TouchEvent};
```

- [ ] **Step 5: Build the framework crate**

Run: `rtk nix develop .#fast -c cargo build -p bmc-system-overlay` Expected: PASS — the client bindings resolve and the
new methods/Dispatch impls type-check.

- [ ] **Step 6: Commit**

```bash
rtk git add system-overlays/bmc-system-overlay Cargo.toml
rtk git commit -F - <<'EOF'
bmc-system-overlay: Add client screen-edge support #BDK-416

- add the ScreenEdge opt-in and on_reveal hook to SystemOverlay
- bind deck_screen_edge_manager_v1 and create an armed auto-hide edge
- dispatch revealed/hidden events and expose drains plus re-arm
EOF
```

---

## Task 6: Framework — drive map/unmap from reveal/hide

Encapsulate the screen-edge lifecycle inside `HostedOverlay` (and the standalone loop) so it reuses the Stage-2 gates:
`revealed` makes `visible` true (first-show render), the overlay returning `visible == false` while revealed runs the
existing `needs_hide`→`hide` path, and `hide` additionally re-arms the edge. `bmc-wasm-host/src/main_loop.rs` is updated
only to prepare hosted overlays for a remapped buffer attach before rendering.

**Files:**

- Modify: `system-overlays/bmc-system-overlay/src/hosted.rs`

- Modify: `system-overlays/bmc-system-overlay/src/standalone.rs`

- [ ] **Step 1: Add screen-edge state to `HostedOverlay`**

In `hosted.rs`, add fields to the struct (after `failed: bool,`):

```rust
    /// `Some(edge)` for a screen-edge overlay; its map/unmap is driven by
    /// reveal/hide events, not directly by `tick`'s `visible`.
    screen_edge: Option<crate::overlay::ScreenEdge>,
    /// True between a `revealed` event and the next hide+re-arm.
    revealed: bool,
```

In `connect`, after `overlay.init();` and before building `Self`, capture the opt-in and arm:

```rust
        let screen_edge = overlay.screen_edge();
        if let Some(edge) = screen_edge {
            client.create_screen_edge(edge)?;
        }
```

Add `screen_edge,` and `revealed: false,` to the `Self { ... }` literal. (The Stage-2 `visible`/`mapped` default to
`false`, which is correct: a screen-edge overlay starts hidden + armed.)

- [ ] **Step 2: Drain reveal/hide in `dispatch`**

In `dispatch` (`hosted.rs`), immediately after the `for ev in self.client.drain_touch()` loop that calls `on_touch`
(hosted.rs:149–151) and before the `drain_released_buffers` loop, add the block below. Any position inside `dispatch`
works (it all runs before `tick`), but pin it here so the placement is not left to guess:

```rust
        if self.screen_edge.is_some() {
            if self.client.take_reveal() {
                self.revealed = true;
                self.overlay.on_reveal();
                self.wants_render = true;
            }
            if self.client.take_hidden() {
                // Compositor-initiated hide: fall into the Stage-2 hide path by
                // clearing revealed; tick() recomputes visible == false.
                self.revealed = false;
            }
        }
```

- [ ] **Step 3: Compute `visible` from reveal state in `tick`**

Replace the `tick` body so a screen-edge overlay's `visible` tracks the reveal, and a dismissed (visible-false) reveal
is honored:

```rust
    /// Run background work; updates visibility, render-want and next-wake.
    pub fn tick(&mut self, now: Instant) {
        let outcome = self.overlay.tick(now);
        self.visible = match self.screen_edge {
            // Screen-edge overlays are on-screen only while revealed, and the
            // overlay can dismiss by returning visible == false.
            Some(_) => self.revealed && outcome.visible,
            None => outcome.visible,
        };
        if self.visible {
            self.wants_render |= outcome.wants_render;
        }
        self.next_wake = outcome.next_wake;
    }
```

(`needs_render`, `needs_hide`, and `mark_rendered` are unchanged: `needs_render` already covers the first-show case
`visible && !mapped`, which fires on reveal; `needs_hide` fires when `visible` drops back to false.)

- [ ] **Step 4: Re-arm in `hide`**

Replace `hide` so a screen-edge overlay re-arms after unmapping:

```rust
    /// Unmap the surface and free export buffers. Called by the host when
    /// `needs_hide` is true. A screen-edge overlay also re-arms its edge so the
    /// next swipe can reveal it again.
    pub fn hide(&mut self, egl: &EglContext) -> anyhow::Result<()> {
        // Ordering is load-bearing: flush the NULL attach before destroying
        // exported buffers so the compositor observes the unmap first.
        self.client.attach_null_buffer()?;
        self.client.roundtrip_after_hide_unmap()?;
        self.target.free_for_hide(egl, &mut self.client)?;
        self.mapped = false;
        self.wants_render = false;
        // Preserve Stage-2 behavior: clear the frame-floor timestamp so a later
        // re-show renders promptly and the hosted/standalone loops stay symmetric.
        self.last_render = None;
        if self.screen_edge.is_some() {
            self.revealed = false;
            self.client.rearm_screen_edge()?;
        }
        Ok(())
    }
```

Also add the remap guard that made the repeated reveal path work on the Deck:

- `LayerSurfaceClient::roundtrip_after_unmap()` drains the compositor response to the NULL-buffer commit before local
  buffer proxies are destroyed.

- `LayerSurfaceClient::roundtrip_after_hide_unmap()` wraps that roundtrip but restores the previous configured size and
  clears render/resize effects from placeholder unmapped configure events (observed as `1x200` for a full-width top
  strip).

- `LayerSurfaceClient::roundtrip_after_resize_unmap(configured_size)` does the same for mapped resize, preserving the
  configured size that triggered the resize so a placeholder unmap configure cannot survive into the next dispatch.

- `LayerSurfaceClient::ensure_ready_for_buffer_attach()` reapplies the saved layer/anchor/size/margins/input-region
  state before the next real buffer attach. Do not wait for a second remap configure here; the next buffer commit
  carries the restored pending layer-shell state.

- `HostedOverlay::prepare_for_render()` calls `ensure_ready_for_buffer_attach()`, then resolves the configured size and
  resizes only if the preserved/resolved size actually changed. Call this from the hosted render path before asking the
  overlay to draw.

- [ ] **Step 5: Add a pure visibility test for the reveal gate**

Add to the `#[cfg(test)] mod tests` block in `hosted.rs` (extending Stage-2's gate tests). The reveal computation lives
in `tick`, which needs a real overlay+client, so test the gate semantics that Stage 2 already exposes plus a focused
documentation test of the rule with a tiny helper:

```rust
    #[must_use]
    fn screen_edge_visible(revealed: bool, overlay_visible: bool) -> bool {
        revealed && overlay_visible
    }

    #[test]
    fn screen_edge_overlay_visible_only_while_revealed() {
        assert!(!screen_edge_visible(false, true), "armed-but-hidden stays unmapped");
        assert!(screen_edge_visible(true, true), "revealed and wanted maps");
        assert!(!screen_edge_visible(true, false), "dismissed while revealed unmaps");
    }
```

> The `screen_edge_visible` helper mirrors the `Some(_)` arm of `tick`; if you change that arm, change this helper to
> match (keep them identical). This is the testable core of the reveal-driven gate; the wiring around it is the same
> Stage-2 path already covered by `overlay_needs_render`/`overlay_needs_hide`.

- [ ] **Step 6: Mirror in `standalone.rs`**

In `run_standalone`, after `overlay.init();` and before the loop, arm if requested:

```rust
    let screen_edge = overlay.screen_edge();
    if let Some(edge) = screen_edge {
        client.create_screen_edge(edge)?;
    }
    let mut revealed = false;
```

Inside the loop, immediately after the `take_configured_size_change` block closes (standalone.rs:73) and before
`let now = Instant::now();` (standalone.rs:75) — i.e. past the touch-drain, released-buffer, and configured-size drains
— add the reveal/hide drains:

```rust
        if screen_edge.is_some() {
            if client.take_reveal() {
                revealed = true;
                overlay.on_reveal();
                pending_render = true;
            }
            if client.take_hidden() {
                revealed = false;
            }
        }
```

Replace the visibility decision (the Stage-2 `if tick.visible { ... } else { ... }` block) so screen-edge overlays gate
on `revealed`:

```rust
        let now = Instant::now();
        let tick = overlay.tick(now);
        let want_visible = match screen_edge {
            Some(_) => revealed && tick.visible,
            None => tick.visible,
        };
        if want_visible {
            if tick.wants_render || !mapped || client.take_needs_render() {
                pending_render = true;
            }
        } else {
            let _ = client.take_needs_render();
            if mapped {
                client.attach_null_buffer()?;
                client.roundtrip_after_hide_unmap()?;
                target.free_for_hide(&egl, &mut client)?;
                mapped = false;
                pending_render = false;
                last_render = None;
                if screen_edge.is_some() {
                    revealed = false;
                    client.rearm_screen_edge()?;
                }
            }
        }
```

Mirror the hosted remap guard in the standalone render path: before drawing a pending frame, call a helper equivalent to
`prepare_for_render()` that invokes `client.ensure_ready_for_buffer_attach()`, resolves the configured size, and resizes
the standalone target only when the resolved size changes. This keeps standalone and hosted overlays aligned after a
NULL-buffer hide/remap cycle.

- [ ] **Step 7: Test + build the framework crate**

Run: `rtk nix develop .#fast -c cargo test -p bmc-system-overlay --lib` Expected: PASS — the Stage-2 gate tests, the new
`screen_edge_visible` test, and the standalone code compiles.

- [ ] **Step 8: Commit**

```bash
rtk git add system-overlays/bmc-system-overlay/src/hosted.rs system-overlays/bmc-system-overlay/src/standalone.rs
rtk git commit -F - <<'EOF'
bmc-system-overlay: Drive map/unmap from edge reveal/hide #BDK-416

- map and render a screen-edge overlay on the revealed event
- gate visibility on the reveal state so dismiss re-hides and re-arms
- free buffers and re-arm the edge on hide, reusing the Stage-2 gates
EOF
```

---

## Task 7: Throwaway screen-edge verification overlay

The Stage-3 analogue of `ValidationOverlay`: a top-anchored panel that arms the top edge, renders a marker on reveal,
and dismisses on tap (which re-arms). It proves the protocol + gesture + framework loop end-to-end and is removed when
Step 4 lands the real quick-settings panel.

**Files:**

- Create: `system-overlays/bmc-system-overlay/src/screen_edge_validation.rs`

- Modify: `system-overlays/bmc-system-overlay/src/lib.rs`

- Create: `system-overlays/screen-edge-validation-overlay/Cargo.toml`

- Create: `system-overlays/screen-edge-validation-overlay/src/main.rs`

- Modify: `bmc-wasm-host/src/overlays.rs`

- Modify: root `Cargo.toml`

- [ ] **Step 1: Write the overlay with a state test**

`system-overlays/bmc-system-overlay/src/screen_edge_validation.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

//! Throwaway top-edge verification overlay. Arms the top edge, draws a marker on
//! reveal, and dismisses on tap (re-arming). Removed when the real swipe panel
//! lands.

use std::time::Instant;

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;

use crate::overlay::{
    InputRegion, LayerConfig, ScreenEdge, SystemOverlay, TickOutcome, TouchEvent,
};

/// Panel height in logical pixels (top strip, full width).
const PANEL_HEIGHT: u32 = 200;

#[derive(Debug, Default)]
pub struct ScreenEdgeValidationOverlay {
    /// True from reveal until a tap dismisses it.
    showing: bool,
    /// Whether the current showing has been drawn at least once.
    rendered: bool,
}

impl SystemOverlay for ScreenEdgeValidationOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig {
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            size: (0, PANEL_HEIGHT),
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            exclusive_zone: 0,
            namespace: "bmc-screen-edge-validation".to_owned(),
            input: InputRegion::Full,
        }
    }

    fn screen_edge(&self) -> Option<ScreenEdge> {
        Some(ScreenEdge::Top)
    }

    fn on_reveal(&mut self) {
        self.showing = true;
        self.rendered = false;
    }

    fn tick(&mut self, _now: Instant) -> TickOutcome {
        TickOutcome {
            visible: self.showing,
            wants_render: self.showing && !self.rendered,
            next_wake: None,
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "panel dimensions fit comfortably in f32 mantissa"
        )]
        let (w, h) = (size.0 as f32, size.1 as f32);
        // Half-transparent panel proving alpha compositing over the live scene.
        r.fill_rect(0.0, 0.0, w, h, Color::from_rgba(20, 40, 120, 200));
        let text = "screen edge OK - tap to dismiss";
        let font = 30.0;
        let tw = r.measure_text(text, font);
        r.draw_text(
            text,
            (w - tw) / 2.0,
            h / 2.0 + font / 3.0,
            font,
            Color::from_rgba(255, 255, 255, 255),
        );
        self.rendered = true;
    }

    fn on_touch(&mut self, event: TouchEvent) {
        if matches!(event, TouchEvent::Down { .. }) {
            self.showing = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_until_revealed_then_dismissed_by_tap() {
        let mut o = ScreenEdgeValidationOverlay::default();
        assert!(!o.tick(Instant::now()).visible, "armed but not revealed");

        o.on_reveal();
        let t = o.tick(Instant::now());
        assert!(t.visible && t.wants_render, "reveal shows and draws once");

        o.on_touch(TouchEvent::Down { id: 0, x: 1.0, y: 1.0 });
        assert!(!o.tick(Instant::now()).visible, "tap dismisses");
    }

    #[test]
    fn arms_the_top_edge() {
        assert_eq!(
            ScreenEdgeValidationOverlay::default().screen_edge(),
            Some(ScreenEdge::Top)
        );
    }
}
```

- [ ] **Step 2: Wire the module + export**

In `lib.rs`, add `mod screen_edge_validation;` (with the other `mod` lines) and
`pub use screen_edge_validation::ScreenEdgeValidationOverlay;` (with the other re-exports).

- [ ] **Step 3: Create the standalone bin**

`system-overlays/screen-edge-validation-overlay/Cargo.toml`:

```toml
[package]
name = "screen-edge-validation-overlay"
version = "0.1.0"
edition = "2024"
authors = ["Braiins Systems s.r.o."]
description = "Throwaway standalone bin for the top-edge verification overlay"

[[bin]]
name = "screen-edge-validation-overlay"
path = "src/main.rs"

[dependencies]
anyhow.workspace = true
bmc-system-overlay.workspace = true

[lints]
workspace = true
```

`system-overlays/screen-edge-validation-overlay/src/main.rs`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_system_overlay::{ScreenEdgeValidationOverlay, run_standalone};

fn main() -> anyhow::Result<()> {
    run_standalone(Box::new(ScreenEdgeValidationOverlay::default()))
}
```

- [ ] **Step 4: Build it into the host for end-to-end testing**

`build_overlays` (`bmc-wasm-host/src/overlays.rs`) builds overlays from a factory `Vec<OverlayFactory>` where
`OverlayFactory = (&'static str, fn() -> Box<dyn SystemOverlay>)` — currently `("offline", ...)` and
`("device-info", ...)`. Add the throwaway overlay as one more entry:

- Extend the existing `use bmc_system_overlay::{HostedOverlay, SystemOverlay};` import to also bring in
  `ScreenEdgeValidationOverlay` (re-exported from `bmc-system-overlay` by Step 2).
- Add an entry to the `factories` vec after the existing two:

```rust
        ("screen-edge", || Box::new(ScreenEdgeValidationOverlay::default())),
```

It sits on `Layer::Overlay` (rank 3), above the device-info/offline overlays, so stacking is correct regardless of build
order. `bmc-system-overlay` is already a `bmc-wasm-host` dependency (`overlays.rs` imports it today), so no `Cargo.toml`
change is needed.

> Throwaway: remove this overlay (and the import) when Step 4 lands the real quick-settings panel. The Stage-2
> device-info/offline overlays are unaffected.

- [ ] **Step 5: Register the bin crate**

In root `Cargo.toml`, add to `members`: `"system-overlays/screen-edge-validation-overlay",`. No workspace.dependencies
entry is needed (nothing depends on the bin).

- [ ] **Step 6: Test, build, clippy, fmt**

Run:

- `rtk nix develop .#fast -c cargo test -p bmc-system-overlay --lib` → PASS (incl. the two new overlay tests)
- `rtk nix develop .#fast -c cargo build -p screen-edge-validation-overlay` → PASS
- `rtk nix develop .#ci -c cargo build -p bmc-wasm-host` → PASS
- `rtk nix develop .#fast -c cargo clippy -p deck-screen-edge-v1 -p bmc-system-overlay -p screen-edge-validation-overlay --tests -- -D warnings`
  → clean
- `rtk nix develop .#ci -c cargo clippy -p bmc-openwrt --tests -- -D warnings` → clean
- `rtk nix fmt` → clean

(Per project rule, do not run `cargo clippy` and `cargo test` in parallel — they share `target/`.)

- [ ] **Step 7: Commit**

```bash
rtk git add system-overlays bmc-wasm-host Cargo.toml
rtk git commit -F - <<'EOF'
bmc-system-overlay: bmc-wasm-host: Add screen-edge verification overlay #BDK-416

- add a throwaway top-edge overlay arming the edge and drawing on reveal
- dismiss on tap, which re-arms for the next swipe
- build it into the host and add a standalone binary
EOF
```

---

## Task 8: On-device verification

No automated GPU tests exist; these are manual on the Braiins Deck, per the design's verification list. Use
`$DEVICE_IP`.

- [ ] **Step 1: Deploy and watch logs**

Build/deploy the host per `docs/nix-device-scripts.md`. Tail the host log for a `Layer surface ready` line with
`namespace=bmc-screen-edge-validation`, confirming the overlay armed (it should NOT map a buffer at startup — the panel
is hidden + armed). Keep the compositor log visible too; the reveal path should log
`top-edge screen-edge swipe consumed` when an armed edge claims the swipe, and
`top-edge reveal gesture activated, but no armed edge consumed it` if the recognizer fires while no edge is armed.

- [ ] **Step 2: Reveal gesture**

Swipe down starting at the very top of the screen (within the top 20% band). Confirm the blue half-transparent panel
appears with `screen edge OK - tap to dismiss`, the live scene shows through its alpha, and a swipe that starts *below*
the top band does NOT reveal it (normal scene interaction instead). Confirm a horizontal swipe at the top edge still
navigates scenes and does not reveal the panel.

- [ ] **Step 3: Neighbor demotion while revealed**

While the panel is revealed, confirm in the host log that scene-swipe neighbor widgets demote to `Dormant` (their
buffers release) and that a scene swipe behind the panel is suppressed. Dismiss with a tap: the panel unmaps, its region
repaints the scene with no stale pixels, neighbors restore to `Prepared`, and the host log shows the edge re-armed (no
further panel renders until the next swipe).

- [ ] **Step 4: Re-arm loop**

Swipe to reveal again, tap to dismiss, several times. Confirm each cycle reveals + dismisses cleanly (the re-arm path
works) with no leaked buffers. Expected host/client log events per cycle: `Screen edge revealed`, `Screen edge hidden`,
`Re-arming screen edge after hide`, and the free-on-hide buffer release path. The panel must still render full-width on
the second and later reveal; a placeholder unmapped configure must not shrink it to a narrow buffer.

- [ ] **Step 5: No MMU-fault / fence regression**

Under the BDK-509 conditions (a widget scene animating while the panel reveals/dismisses repeatedly), confirm no
scene-freeze MMU faults — the overlay rides the host GL-fence handoff and the compositor alpha-blend waits on the fence.
If faults appear, capture the log and stop; do not paper over with a second lock.

- [ ] **Step 6: Record results**

Note pass/fail per step in the MR description. Reveal-latency measurement (allocate-on-reveal) and the short reveal
animation are deferred to Step 4 (the real swipe panel); not in scope here.

---

## Self-review notes

- **Spec coverage (design Step 3):** vendored+renamed `deck_screen_edge_v1` with the `revealed`/`hidden` extension (Task
  1), compositor `Dispatch` (Task 2), the top-edge gesture (Task 4), and the neighbor→`Dormant` demotion (Task 3). The
  framework client support and a verification overlay (the user's explicit asks) are Tasks 5–7. The real swipe panel
  - reveal animation are Step 4, intentionally out of scope.
- **Two interfaces, not two protocols:** `deck_screen_edge_manager_v1` is the registry-global factory;
  `deck_auto_hide_screen_edge_v1` is the per-surface object carrying `activate`/`deactivate` and `revealed`/`hidden`.
  Standard Wayland factory split (mirrors `deck_widget_manager_v1` + `deck_widget_surface_v1`).
- **Contract:** hide always `activate()`→`hidden`; show always `revealed`. Dismiss re-arms via `activate()`. This gives
  a single path each way and is exercised end-to-end by the verification overlay's reveal→tap→re-arm loop.
- **Host-loop scope:** the screen-edge lifecycle is encapsulated in `HostedOverlay` + `LayerSurfaceClient`, reusing
  Stage-2's `needs_render`/`needs_hide`/`hide` gates. The host render path only calls `prepare_for_render()` before
  drawing so a previously hidden layer surface can reapply its layer-shell state before the next buffer attach.
- **Suppression reuse:** `neighbors_suppressed() = fullscreen_blocker_active() || any_screen_edge_revealed()` OR's the
  new signal into Stage-2's existing demotion/suppression machinery; with no edge revealed, behavior is byte-for-byte
  the Stage-2 behavior.
- **Gesture precedence:** top-edge reveal is checked before horizontal scene drag and allows
  `EDGE_MAX_X_DEVIATION = 150px` of horizontal drift. A mostly horizontal motion in the top band still becomes scene
  drag, but a downward reveal sample (`dy >= 40`) that stays within the 150px drift budget is claimed as reveal even
  though its `dx` can exceed `DRAG_DEAD_ZONE`. `GestureState` reports the motion activation; the compositor only claims
  the forwarded touch sequence when an armed top edge consumes it.
- **Supported edge:** top only (a downward swipe), matching the approved spec's swipe-from-top reveal. Bottom and
  left/right are deliberately omitted from the protocol, framework, and compositor — left/right would be horizontal
  gestures that conflict with the scene swipe (which can begin anywhere, including at a screen edge), and no Stage-3
  overlay needs bottom. The protocol fixes `top = 1` so a bottom edge can be added non-breaking when a spec'd overlay
  requires it.
- **Stage-2 dependency:** this plan references post-Stage-2 symbols (`is_fullscreen_blocker`,
  `fullscreen_blocker_active`, `Layer::Top` boot screen, `Layer::Overlay` reserved, `TickOutcome.visible`, the rewritten
  `HostedOverlay`/`run_standalone`). Verify they exist before starting (see the dependency section).
- **Throwaway:** the verification overlay + its bin + the `build_overlays` entry are removed when Step 4 lands the real
  quick-settings panel, exactly as Stage 1's `ValidationOverlay` was retired in Stage 2.
- **Hot-zone fraction:** `EDGE_HOT_ZONE_FRACTION = 0.20` is the spec'd top-edge band (96 px on the 480 px-tall display).
  Keep it fixed unless the spec is amended again. `EDGE_ACTIVATION_DY = 40px` is the practical activation distance to
  verify on device in Task 8. `EDGE_MAX_X_DEVIATION = 150px` accepts imperfect downward swipes without weakening the
  edge-start requirement.
- **`already_constructed` error:** enforced — the manager rejects a second `get_auto_hide_screen_edge` for a surface
  that already has one (Task 2 posts `already_constructed`), so the declared error is not decorative.

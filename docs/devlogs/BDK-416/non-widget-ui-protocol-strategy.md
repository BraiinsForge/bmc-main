# Non-widget UI protocol strategy

**Ticket:** BDK-416 **Date:** 2026-06-14 **Status:** decision recorded **Related spec:**
`docs/superpowers/specs/2026-06-07-system-overlays-design.md`

## Decision

Use `deck_widget` only for scene widgets.

Use Wayland shell and input protocols for non-widget UI components:

| Component                                                                                        | Protocol strategy                                                                           |
| ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- |
| System overlays, setup screens, alarms, quick settings, notifications, passive status indicators | `wlr-layer-shell` surfaces                                                                  |
| Swipe-from-top quick settings reveal                                                             | `wlr-layer-shell` plus a Deck-owned `deck_screen_edge_v1` extension                         |
| On-display keyboard visual surface                                                               | `wlr-layer-shell`                                                                           |
| On-display keyboard text routing                                                                 | `input-method-v2` plus `text-input-v3`; add `virtual-keyboard-v1` only for raw-key fallback |

This is BDK-416 option B with one scoped extension: standard Wayland protocols are used where they model the behavior,
and a Deck protocol is added only for the top-edge auto-hide/reveal behavior that is not available in Smithay as a
ready-made server implementation.

## Rationale

`deck_widget` is the widget protocol. It carries widget surface registration, compositor-provided size and viewport
configuration, widget params, settings delivery, and widget action requests. Scene placement itself stays owned by the
scene configuration and compositor, not by the protocol. Shell components still need different semantics: stacking above
the active scene, edge or corner anchoring, exclusive zones, explicit input regions, keyboard focus policy, and
text-input routing. Adding those concepts to the widget protocol would make it responsible for shell behavior it was not
designed to model.

`wlr-layer-shell` already models shell surfaces. It gives us layers, anchors, exclusive zones, keyboard interactivity,
and normal pointer/touch hit-testing through the compositor. Smithay has server-side support for it, so the work is
wiring and policy rather than protocol design.

The on-display keyboard should not use the widget protocol for text entry. Wayland already separates the keyboard visual
surface from text routing: the keyboard is a layer-shell client for display, an input-method client for text state and
commit flow, and optionally a virtual-keyboard client for raw key fallback.

## On-display keyboard

A future on-display keyboard is a system overlay with two separate jobs:

- draw and receive touches on the keyboard surface
- commit text or editing operations into the focused text field

The visual keyboard surface uses `wlr-layer-shell`. When the user taps a key such as `A`, the keyboard handles the touch
locally, resolves it through its own layout state, and sends the resulting text through `input-method-v2` to the
compositor. The compositor then forwards the committed text to the focused text field through `text-input-v3`.

`virtual-keyboard-v1` is not the primary typing path. It injects raw key events into the seat and is useful for fallback
or non-text keys such as arrows, escape, or enter. Normal text entry should use text commits so layout, composition,
surrounding text, hints, and future IME behavior remain expressible.

## Screen-edge extension

The swipe-from-top panel needs compositor-owned gesture detection. A normal layer-shell surface can be placed at the top
edge, but it does not define an auto-hide edge trigger.

Use a vendored and renamed protocol, `deck_screen_edge_v1`, forked from `kde-screen-edge-v1`.

The fork is intentionally renamed because the contract is not KDE's contract:

- add `revealed` and `hidden` events so hosted overlays know when to animate, render, or free resources
- allow hidden overlays to hold no buffer, using a NULL-buffer commit to unmap the surface
- keep layer-shell responsible for placement while screen-edge owns arming, hiding, and reveal triggering

The protocol crate should live at the workspace root beside `bmc-widget-protocol`, not under the overlay crate group,
because both the compositor and overlay framework depend on it.

## Runtime model

System overlays are privileged native clients, not WASM widgets. For the current memory-constrained target they compile
into `bmc-wasm-host`, but each overlay still opens its own Wayland connection so it remains a separate client from the
compositor's perspective.

Hosted overlays borrow the host's shared renderer only for the duration of a render callback. This shares the expensive
GL context and font cache without making renderer ownership part of the overlay API.

The standalone mode remains a supported shape for later: it owns its Wayland connection, renderer, event loop, and the
GPU render lock.

## Input policy

Layer-shell input regions define whether an overlay consumes input.

- fullscreen setup or alarm overlays use a full input region while visible
- passive status indicators use an empty input region so touches fall through
- the quick-settings panel accepts input only over the revealed panel
- hidden or unmapped overlays accept no input

The top-edge reveal gesture belongs to the compositor. A touch that starts in the top hot zone is initially forwarded to
the focused surface while the vertical recognizer evaluates. If the downward reveal gesture activates, the compositor
sends `wl_touch.cancel` to the previous recipient and reveals the panel. Touches outside the hot zone never participate
in the reveal gesture.

Keyboard modality should come from the standard protocols: layer-shell `keyboard_interactivity` for shell focus policy,
and input-method/text-input protocols for OSK text flow.

## Memory and GPU constraints

Hidden overlays must not retain fullscreen buffers. On hide, the overlay commits a NULL buffer and frees its DMA-BUFs.
The compositor must track layer-surface buffers separately from widget buffers, release replaced or unmapped buffers,
evict imported textures on NULL-buffer unmap, and damage the vacated region so the scene repaints where the overlay was.

Hosted overlays must publish buffers through the same host GL-fence handoff discipline used by widgets. Standalone
overlays must take `/run/bmc-gpu-render.lock` around their GPU submits.

## Follow-up implementation shape

The approved system-overlays design decomposes implementation into:

1. compositor `wlr-layer-shell` support, layer-surface compositing, buffer tracking, and a `bmc-system-overlay`
   framework
2. initial layer-shell overlays that need no compositor signal, such as offline and startup-IP indicators
3. `deck_screen_edge_v1`, top-edge gesture recognition, and reveal/hide events
4. the swipe-from-top quick-settings overlay
5. on-display keyboard work using layer-shell plus input-method/text-input protocols when that feature is scheduled; add
   virtual-keyboard support only when raw-key fallback is needed

BDK-416 is closed by this protocol decision. Implementation belongs in the follow-up tickets for the concrete overlays
and input components.

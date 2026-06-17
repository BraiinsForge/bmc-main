# Overlay Protocols

System overlays use two small Wayland protocols beyond `wlr-layer-shell`, both vendored as their own crates at the
workspace root (`deck-screen-edge-v1/`, `deck-settings-v1/`) beside `bmc-widget-protocol`. They are shared between the
compositor and the overlay framework, so they do not live under `system-overlays/`. Each crate carries the `.xml` and
generates both server and client bindings with `wayland_scanner` (`generate_server_code!` / `generate_client_code!`),
matching the `bmc-widget-protocol` convention.

Both are forks with deliberately renamed interfaces. The `deck_` prefix follows the `deck_widget_v1` precedent of not
impersonating someone else's protocol: the contracts differ from their upstreams, so keeping the upstream interface
names would mislead the next reader into assuming upstream semantics. The compositor-side dispatch for both is in
[`compositor-integration.md`](compositor-integration.md); the client-side binding is in [`framework.md`](framework.md).

## `deck_screen_edge_v1`

Forked from `kde-screen-edge-v1`. It associates an auto-hide reveal action with a screen edge for a layer surface: the
surface is hidden, and the compositor reveals it on an edge swipe. KDE does the gesture detection in the compositor with
the client uninvolved in detection; this protocol adopts the same division.

**The fork's divergences from upstream:**

- The surface is **hidden by default and holds no buffer while hidden**. Upstream assumes the surface keeps its buffer
  so the compositor can reveal it instantly; this fork trades that instant reveal for zero allocation while hidden
  (allocate-on-reveal, free-on-hide).
- Two events, `revealed` and `hidden`, are **added** so the hosted overlay knows when to allocate/animate and when to
  free buffers — upstream un-hides silently.
- Left/right borders are **omitted**; only top and bottom are defined, because horizontal gestures conflict with scene
  navigation.

### `deck_screen_edge_manager_v1` (version 1)

| Member                                           | Kind    | Args                                                      | Notes                                                                                                                                                                                     |
| ------------------------------------------------ | ------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `destroy`                                        | request | —                                                         | Destructor. Does not destroy objects created with the manager.                                                                                                                            |
| `get_auto_hide_screen_edge(id, border, surface)` | request | `id: new_id`, `border: uint(enum)`, `surface: wl_surface` | `invalid_border` for an out-of-range border; `invalid_role` unless `surface` already has the layer-surface role; `already_constructed` if `surface` already has an auto-hide screen edge. |

The `border` enum is `top = 1`, `bottom = 2`. A top reveal is a downward gesture; a bottom reveal an upward one. The
`error` enum is `invalid_border` (border out of range), `invalid_role` (surface lacks the layer-surface role), and
`already_constructed` (the surface already has an auto-hide screen edge — per-surface uniqueness).

### `deck_auto_hide_screen_edge_v1` (version 1)

| Member       | Kind    | Notes                                                                                                                                  |
| ------------ | ------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `destroy`    | request | Destructor.                                                                                                                            |
| `deactivate` | request | Disarm the edge and request the surface be shown; the compositor emits `revealed`.                                                     |
| `activate`   | request | Arm the edge and request the surface be hidden; the compositor emits `hidden`. A spent (already-triggered) edge is re-armed with this. |
| `revealed`   | event   | The edge gesture fired (or `deactivate` was called). The client allocates, renders, and attaches a buffer. The arming is now spent.    |
| `hidden`     | event   | The client attaches a NULL buffer and frees its DMA-BUFs.                                                                              |

**Responsibility split.** Layer-shell owns *placement* — the layer surface's anchor/size/exclusive-zone fix where the
panel lives; the `wl_surface` argument to `get_auto_hide_screen_edge` is only an association. screen-edge owns
*visibility and the trigger* — arming, hiding, and reveal. This is why `get_auto_hide_screen_edge` raises `invalid_role`
unless the layer-surface role is already set.

## `deck_settings_v1`

Re-homed from the BDK-343 btc-prague branch, where it lived inside `bmc-widget-protocol` and was bound by a settings
*widget*. Here it is a standalone protocol bound only by the settings-tray layer-shell overlay. It is a lightweight
compositor-relayed IPC for system state the overlay must not touch directly: the target platform (OpenWrt/ARMv7) has no
D-Bus/PipeWire/UPower, and bmc owns the hardware drivers, so the overlay relays control through the compositor.

The BDK-343 `dismiss` request is **dropped** in the re-home: hide is owned by `deck_screen_edge_v1` (`activate()` →
`hidden`), so a compositor-driven dismiss is redundant.

### `deck_settings_v1` (version 1)

| Member                  | Kind    | Args                  | Notes                                                                                                                                                                    |
| ----------------------- | ------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `set_brightness(value)` | request | `value: uint` (0–100) | bmc applies it night-mode-aware and reports the effective value back via `brightness`.                                                                                   |
| `reconfigure_wifi`      | request | —                     | Put the device into WiFi setup mode (open AP + captive portal). One-way; the device leaves setup mode on its own once configured from the phone. Progress via `wifi_ap`. |
| `destroy`               | request | —                     | Destructor.                                                                                                                                                              |
| `brightness(value)`     | event   | `value: uint` (0–100) | Effective brightness. Emitted on bind and on every change, including the night-mode value while night mode is active.                                                    |
| `wifi_ap(ssid)`         | event   | `ssid: string`        | Setup-AP SSID. Non-empty means setup mode is active; empty means inactive. Emitted on bind and on change.                                                                |

**Responsibility split.** The overlay sends `set_brightness` / `reconfigure_wifi`; the compositor forwards them to bmc
over the existing action channel and emits `brightness` / `wifi_ap` back when bmc broadcasts. The compositor caches the
last brightness and SSID so a late-binding overlay receives current values immediately on bind. Brightness is applied by
bmc night-mode-aware — bmc owns the *effective* value — which is exactly why the overlay routes through bmc rather than
writing the backlight sysfs itself.

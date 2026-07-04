# The Concrete Overlays

Three overlays ship today, each a small crate under `system-overlays/` implementing `SystemOverlay` (see
[`framework.md`](framework.md)). This document covers what each shows, when it maps and dismisses, where its data comes
from, and its platform gating.

## Device info (`bmc-overlay-device-info`)

A full-screen startup overlay that mirrors the legacy boot-status screen: it maps immediately at operational startup,
shows WiFi/IP connection progress, then a success or failure result, then unmaps for the rest of the session. It is
purely observational — it watches saved WiFi config and IP state and never drives a connect flow.

`LayerConfig::fullscreen` → `Layer::Top`, full input region. It blocks scene touch while shown.

Its state is a four-phase machine driven by `tick`, with the IP and SSID read from the connectivity prober's
`snapshot_if_changed` (see [`framework.md`](framework.md)); ticks wake at `POLL = 1s`. Until the first snapshot is
published the overlay treats the state as "no IP yet", so the `WAIT_FOR_IP` deadline keeps running:

| Phase        | Shown                                 | Exit                                                                           |
| ------------ | ------------------------------------- | ------------------------------------------------------------------------------ |
| `Connecting` | the station SSID, "waiting for IP"    | a routable IPv4 appears → `Success`; else after `WAIT_FOR_IP` (20s) → `Failed` |
| `Success`    | "you're connected" and `http://<ip>/` | after `SUCCESS_VISIBLE_FOR` (10s) → `Done`                                     |
| `Failed`     | a connection-problem message          | after `FAILURE_VISIBLE_FOR` (5s) → `Done`                                      |
| `Done`       | nothing (unmapped, no further wake)   | terminal                                                                       |

In `Success` the last-known IP is held even through a transient DHCP loss, so the screen does not flicker. A touch
anywhere dismisses immediately (jumps to `Done`).

## Offline (`bmc-overlay-offline`)

A passive bottom-right "OFFLINE" indicator, mapped only while the device has no routable IPv4 and unmapped again when
connectivity returns (and re-mapped if it drops again).

`LayerConfig::bottom_right("bmc-overlay-offline", (160, 48))` → `Layer::Bottom` with **no input region**, so touches in
its corner fall through to whatever is behind it. The `Bottom` layer means a full-screen `Top` or `Overlay` overlay
occludes it.

`tick` polls the prober's `snapshot_if_changed` on every wake (`POLL` = 2s) and keeps the state derived from the last
changed snapshot; the overlay is visible exactly when a published snapshot holds no routable IPv4 (before the first
snapshot the chip stays hidden). It draws a content-tight box at the bottom-right corner (opaque black background, red
label), with the rest of the surface transparent. It takes no touch input.

## Settings tray (`bmc-overlay-settings-tray`)

The swipe-from-top quick-settings panel: a brightness slider, WiFi station info, and hold-to-confirm WiFi
reconfigure/reconnect buttons. It is the only overlay that uses both vendored protocols. It is ported from the BDK-343
`settings-stub` widget, translated to the native `bmc-render` tree.

Its `LayerConfig` is built by hand: `Layer::Overlay`, anchored to all four edges, **full** input region (the tray is
full-screen and blocks scene swipes while it is up). `screen_edge()` returns `ScreenEdge::Top` and `uses_settings()` is
`true`.

### Reveal and dismiss

The panel is armed to the top edge: hidden (no buffer) until the compositor's top-edge swipe reveals it. On reveal
(`on_reveal`) it resets its FSM and touch tracking and starts the slide. The slide is **blit-only** — the panel is laid
out and painted once into the GPU cache and re-blitted at the animation offset, never re-laid-out per frame (see
[`framework.md`](framework.md)); `SLIDE_MS = 180` ms, eased.

It dismisses on either of:

- an **upward swipe** that travels up at least `DISMISS_DY` (60 px) and is mostly vertical — classified in `dismiss.rs`,
  distinct from the horizontal slider drag;
- **inactivity** after `INACTIVITY_TIMEOUT` (15 s) with no touch.

Dismiss runs the slide in reverse and reports `visible = false` only once it completes, at which point the framework
unmaps and re-arms the edge.

### Controls and data

- **Brightness slider** — a `ProgressBar` tree node with a `touch_key`; the drag fraction maps to a brightness value
  sent as `SettingsRequest::SetBrightness`, throttled to `BRIGHTNESS_SEND_INTERVAL` (80 ms) during a drag with a final
  value flushed on finger-up. The compositor's `brightness` event (`on_brightness`) updates the displayed value.
- **WiFi info** — hostname (read once from `/proc/sys/kernel/hostname`) plus the configured SSID, current IP, and a
  signal-strength icon, all from the connectivity prober's `snapshot_if_changed`; the signal icon is chosen from dBm
  thresholds. The versioned read is polled on every tick (free while the snapshot is unchanged, even at the ~30 Hz
  animation cadence); `NETWORK_REFRESH` (2 s) is the idle wake cadence.
- **WiFi setup view** — when `on_wifi_ap` reports a non-empty setup-AP SSID, the panel replaces the station info and
  buttons with a setup badge and the AP SSID for the user to join from their phone.
- **Reconfigure WiFi** — a hold-to-confirm button (`HOLD` = 3 s) that sends `SettingsRequest::ReconfigureWifi`; the FSM
  advances through holding/pending/active states from the `wifi_ap` event, with a timeout and error label if setup never
  starts.
- **Reconnect WiFi** — a separate hold-to-confirm button that fire-and-forget spawns a detached shell sequence (pulse
  the `WIFI-RESET` GPIO, then bounce the WiFi stack: `wifi down; wifi up`). The child is reaped so it does not zombie;
  there is no completion event.

### Platform gating

The reconfigure and reconnect buttons are only rendered where the platform supports them: `wifi_reconfig_supported` is
true for `Product::Bmc100` and `Product::Bfm100` only. BMM boards (ESP32 AP) hide both buttons. The panel also adapts
its layout to display shape (round vs. wide vs. narrow rectangular).

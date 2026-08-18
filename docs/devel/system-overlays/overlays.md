# The Concrete Overlays

Five overlays ship today, each a small crate under `system-overlays/` implementing `SystemOverlay` (see
[`framework.md`](framework.md)). This document covers what each shows, when it maps and dismisses, where its data comes
from, and its platform gating.

## Device info (`bmc-overlay-device-info`)

The full-screen transient boot and setup screens: the first-boot setup flow (AP SSID + QR, connecting, connected,
device-setup IP QR, completed, errors), WiFi reconfiguration, and the operational-boot connect-info sequence. It is a
port of the stable-26.02 `display_tasks` screens onto the overlay framework.

`LayerConfig::fullscreen` with the layer lowered to `Layer::Bottom`, full input region. It blocks scene touch while
shown, but sits below a firing alarm (`Top`), the firmware-upgrade splash (`Top`), and the settings tray (`Overlay`).
Because it is a full-screen surface above `Background`, the compositor's `is_fullscreen_blocker` predicate suppresses
scene swipes for as long as any screen is up — the scenes handoff is purely the unmap (see
[`compositor-integration.md`](compositor-integration.md)).

### Inputs

bmc owns the lifecycle and drives the overlay over `deck_device_info_v1` (see [`protocols.md`](protocols.md)):
`device_state` selects the flow, `setup_progress` steps the setup flow, and `access_point` carries the setup-AP SSID and
wizard URL (so the overlay hard-codes no AP addressing). All three are replayed on bind. The displayed device address
comes from the connectivity prober's `station_ipv4` — the pick that excludes AP-mode interfaces, so the setup AP's own
address never counts as an uplink. Until the first `device_state` event the overlay stays unmapped rather than guess a
flow.

Every screen-hold timer lives in the overlay; bmc emits transitions the moment they happen (recovery policies — the
no-IP factory reset and the no-AP reboot — stay in bmc, which broadcasts `unexpected_error` before acting).

### Flows

- **Operational boot**: connecting (SSID, "waiting for IP") → connect-info (IP + QR, 10 s) on an address, or failure (5
  s) after 20 s without one → unmap. A touch dismisses this flow immediately; a post-upgrade boot skips it or opens on
  the upgrade-success screen (see "Opening after an upgrade" below — that applies to this flow only, never the setup
  screens).
- **First boot** (`factory_default`): setup-start (AP SSID + QR of the wizard URL; a placeholder until `access_point`
  arrives) → `connecting_to_wifi` → connected (5 s) → setup connect-info (device-setup IP QR) → `device_setup_success` →
  completed (5 s) → unmap. A `wifi_connection_failed` shows the error for 5 s and returns to setup-start (the AP is
  still up). Setup screens ignore touch — dismissing them would leave a blank screen mid-wizard.
- **SetupPending boot** (configured but unfinished): connecting, self-advancing to the setup connect-info when the
  station address appears; bmc's watchdog factory-resets if none comes.
- **WiFi reconfiguration**: the same setup flow, entered when `device_state` flips to `wifi_reconfiguration`; on
  `wifi_reconfig_success` the connected screen shows 5 s and unmaps straight to scenes (no connect-info). The
  setup-start screen auto-hides after 8 minutes with the AP left up; a later setup event revives the flow.
- **Unexpected error**: sticky full-screen error; bmc recovers on its own (factory reset or reboot).

In the operational connect-info the last-known IP is held through a transient DHCP loss, so the screen does not flicker.
The screens render through the `bmc-render` tree pipeline with the six legacy init-setup SVG icons embedded at build
time; every screen has a gallery cell (`overlays.scene.rs`).

### Opening after an upgrade

The overlay also binds `deck_upgrade_v1` (`uses_upgrade()` is `true`) because a boot that follows an upgrade is not an
ordinary boot. It reacts only to a terminal *success* snapshot, which is what marks this startup as post-upgrade, and
that decides how the operational flow opens:

- **After a firmware upgrade** the flow opens on the "Update Finished" screen for `HOLD` (5 s) and then starts
  `Connecting`. This overlay owns that screen: `bmc-overlay-upgrade` deliberately shows no firmware success, so the
  confirmation and the connect window are one uninterrupted sequence rather than two surfaces taking turns. A touch
  skips ahead to `Connecting` instead of handing off to the scenes — the screen is an interstitial, not the end of a
  flow.
- **After a package activation restart** the flow is skipped entirely (`Done`). Only the compositor restarted — the
  network never dropped — so a connection screen would be stale noise.

The snapshot's `remaining` dwell is ignored: like every other screen here, this one is timed by the overlay.

Which of the two paths runs is decided by the runner, not by the wire: the device-info events are drained before the
snapshot is applied in `tick`, whichever order the compositor replayed them in. So on a post-upgrade boot the
`device_state` has already opened the connect screen, and the snapshot switches it to the upgrade screen on the spot,
gated on `Connecting` so an upgrade finishing minutes later cannot resurrect it. The *latch* covers the other order,
where the snapshot lands with no connect screen to replace: only the operational entry consumes it, so a success
arriving mid-setup cannot disturb the setup screens.

## Offline (`bmc-overlay-offline`)

A passive bottom-right "OFFLINE" indicator, mapped only while the device has no routable IPv4 and unmapped again when
connectivity returns (and re-mapped if it drops again).

`LayerConfig::bottom_right("bmc-overlay-offline", (160, 48))` → `Layer::Background` with **no input region**, so touches
in its corner fall through to whatever is behind it. `Background` is the lowest rank, so every other overlay draws over
it; it still paints above the scene, so the indicator stays visible over the clock.

`tick` polls the prober's `snapshot_if_changed` on every wake (`POLL` = 2s) and keeps the state derived from the last
changed snapshot; the overlay is visible exactly when a published snapshot holds no routable IPv4 (before the first
snapshot the chip stays hidden). It draws a content-tight box at the bottom-right corner (opaque black background, red
label), with the rest of the surface transparent. It takes no touch input.

## Settings tray (`bmc-overlay-settings-tray`)

The swipe-from-top quick-settings panel: ± brightness and volume controls, a night-mode toggle, and hold-to-confirm
restart and WiFi reconfigure buttons over the WiFi station info. It is the only overlay that uses both vendored
protocols. It is ported from the BDK-343 `settings-stub` widget, translated to the native `bmc-render` tree.

Its `LayerConfig` is built by hand: `Layer::Overlay`, anchored to all four edges, **full** input region (the tray is
full-screen and blocks scene swipes while it is up). `screen_edge()` returns `ScreenEdge::Top` and `uses_settings()` is
`true`.

### Reveal and dismiss

The panel is armed to the top edge: hidden (no buffer) until the compositor's top-edge swipe reveals it. On reveal
(`on_reveal`) it resets its FSM and touch tracking and starts the slide. The slide is **blit-only** — the panel is laid
out and painted once into the GPU cache and re-blitted at the animation offset, never re-laid-out per frame (see
[`framework.md`](framework.md)); `SLIDE_MS = 180` ms, eased.

It dismisses on any of:

- an **upward swipe** that travels up at least `DISMISS_DY` (60 px) and is mostly vertical — classified in `dismiss.rs`,
  distinct from a horizontal drag across the controls;
- **inactivity** after `INACTIVITY_TIMEOUT` (15 s) with no touch;
- **preemption** — the compositor reports (via `deck_settings_v1.preempted`) that a modal full-screen overlay, such as a
  firing alarm, has mapped below the tray. `on_preempted(true)` runs the same dismiss so the tray never sits on top of
  it. This is generic: any full-screen modal overlay triggers it, so the tray does *not* bind each such feature's
  protocol. See the modal-preemption policy in [`compositor-integration.md`](compositor-integration.md).

Dismiss runs the slide in reverse and reports `visible = false` only once it completes, at which point the framework
unmaps and re-arms the edge. A preemption while the tray is already hidden is a no-op — the surface stays unmapped
because a screen-edge overlay is only shown while both revealed *and* `tick`-visible (see
[`framework.md`](framework.md)).

### Controls and data

- **Brightness and volume** — a ± pair of round buttons each, stepping the value by `STEP` (10) and clamping to
  `ui::MIN_BRIGHTNESS`..100 and 0..100 respectively, sent as `SettingsRequest::SetBrightness` / `SetVolume`. The
  compositor's own event (`on_brightness`, `on_volume`) updates the displayed value, except during the
  `STEP_ECHO_SETTLE` (300 ms) window after a step, where a stale echo would otherwise bounce the value back.
- **WiFi info** — the configured SSID, the current IP, and a signal-strength icon from the connectivity prober's
  `snapshot_if_changed`, plus the hostname, read once from `/proc/sys/kernel/hostname`; the signal icon is chosen from
  dBm thresholds. The versioned read is polled on every tick (free while the snapshot is unchanged, even at the ~30 Hz
  animation cadence); `NETWORK_REFRESH` (2 s) is the idle wake cadence.
- **Where the addresses render** — the wide tier's header carries the IP, the hostname, and a QR code of `http://<ip>`;
  the compact tiers have room for one address, so they head the panel with the IP alone (`---` while unknown) and drop
  the hostname, keeping SSID and signal on the bottom line.
- **WiFi setup view** — when `on_wifi_ap` reports a non-empty setup-AP SSID, the panel replaces the station info with a
  setup badge and the AP SSID for the user to join from their phone, and hides the reconfigure button. The other
  controls stay.
- **Reconfigure WiFi** — a hold-to-confirm button (`HOLD` = 3 s) that sends `SettingsRequest::ReconfigureWifi`; the FSM
  advances through holding/pending/active states from the `wifi_ap` event, with a timeout and error label if setup never
  starts.

### Platform gating

The reconfigure button follows the compositor's `caps.wifi_setup` on v2. On v1 it falls back to
`wifi_reconfig_supported`, true for `Product::Bmc100` and `Product::Bfm100` only, so BMM boards (ESP32 AP) hide it. The
panel also adapts its layout to display shape (round vs. wide vs. narrow rectangular).

## Alarm (`bmc-overlay-alarm`)

The full-screen screen shown while a clock alarm is ringing: the alarm's scheduled time (large), its label, a **Stop
Alarm** button, and — when snoozing is still allowed — a **Snooze** button. It is purely a UI relay for the alarm domain
in `bmc`; it neither schedules nor sounds the alarm (that is the scheduler and audio subsystem) and holds no timers of
its own.

`LayerConfig::fullscreen` → `Layer::Top`, full input region, so it covers the scene and captures all touch while up.
`uses_alarm()` is `true`; it does not use the screen edge or `deck_settings_v1`. Because it is a full-screen `Top`
surface, the compositor treats it as a modal blocker: it suppresses scene navigation and preempts the settings tray (see
[`compositor-integration.md`](compositor-integration.md)) with no per-overlay wiring.

### Map, dismiss, and snooze gating

Visibility is a single `Option<Ring>`; `tick` reports `visible` exactly while it is `Some`, so the overlay is purely
event-driven with no timed wake:

- **`on_alarm_ring(time, label, snooze_allowed)`** (the `deck_alarm_v1.alarm_ringing` event) fills the ring state and
  maps the surface. `snooze_allowed` is decided in `bmc` — `not_allowed` when the alarm has no snooze options *or* its
  per-firing snooze count has reached the configured limit — and hides the Snooze button.
- A **Stop Alarm** tap queues `AlarmRequest::Dismiss`; a **Snooze** tap queues `AlarmRequest::Snooze`. The framework
  drains these after `render` and sends them over `deck_alarm_v1`; `bmc` acts and the resulting stop comes back as the
  `alarm_stopped` event.
- **`on_alarm_stop`** (the `deck_alarm_v1.alarm_stopped` event) clears the ring state so the surface unmaps. It is sent
  for any stop the overlay did not initiate — timeout, a dismiss from the web UI, or the compositor's no-overlay
  fallback.

The compositor keeps a **no-overlay / crash fallback**: if an alarm rings with no live overlay bound (or the overlay
dies mid-ring), it auto-dismisses after a short grace, and any touch dismisses it immediately. That watchdog lives in
[`compositor-integration.md`](compositor-integration.md).

## Upgrade progress (`bmc-overlay-upgrade`)

On-device feedback for a running upgrade: the current stage, a progress bar while one is meaningful, and a terminal
result screen — "Update Failed" for either kind, and "Update Finished" for a package run. The post-reboot firmware
success screen belongs to the device-info overlay (see above), so this one drops a firmware `Succeeded` snapshot rather
than mapping for it. Like the alarm it is a pure relay — bmc owns every upgrade decision and the overlay renders the
display projection it receives over `deck_upgrade_v1` (see [`protocols.md`](protocols.md)).

The crate exports **two** `SystemOverlay` implementations, `UpgradeOverlay::firmware()` and
`UpgradeOverlay::packages()`, because `LayerConfig` is static and the two presentations differ in every field that
matters:

|           | Firmware                                  | Packages                                       |
| --------- | ----------------------------------------- | ---------------------------------------------- |
| Placement | full-screen (`LayerConfig::fullscreen`)   | bottom-right, `PACKAGE_SURFACE_SIZE` (384×192) |
| Layer     | `Top`                                     | `Bottom`                                       |
| Input     | full                                      | none                                           |
| Effect    | modal: blocks the scene for the whole run | passive: widgets stay visible and interactive  |

Both clients bind the protocol and receive every snapshot; each maps only for its own kind and clears its view when a
snapshot of the *other* kind arrives. The inactive client stays unmapped and holds no DMA-BUFs, so the split costs
nothing while idle. A firmware-containing run uses the firmware surface even when it also carries packages — anything
that reboots the device blocks the screen.

Making one surface reconfigure its size, anchors, layer, and input policy at runtime was the alternative. It was
rejected: it would add runtime surface reconfiguration to the overlay framework for a single caller.

Because the firmware surface is a full-screen `Top` surface, the compositor treats it as a modal blocker on the same
generic policy as the alarm — suppressed scene navigation and a preempted settings tray, with no per-overlay wiring (see
[`compositor-integration.md`](compositor-integration.md)).

### Map, progress, and dismiss

Visibility is a single `Option<UpgradeView>`, filled from `on_upgrade_state` and cleared when the run ends:

- **Running** shows the phase label ("Verifying firmware", "Verifying packages", "Preparing update" before the first
  phase) and a bar whose mode follows the snapshot: determinate when a download reports a total, indeterminate when it
  reports only bytes downloaded. An animating bar wakes at `ANIMATION_FRAME` (100 ms) — package realization can run for
  minutes under CPU and flash load, so 10 fps is enough and deliberately cheap.
- **Terminal** (`Succeeded` / `Failed`) carries a `remaining` interval from bmc; the overlay stores it as a deadline and
  unmaps when `tick` passes it. Repeated terminal snapshots keep the original deadline, so a coalesced re-send does not
  extend the screen. A firmware `Succeeded` is the one snapshot neither surface shows — the device-info overlay opens on
  it instead.
- **Touch** changes nothing on either surface. The firmware surface keeps a full input region because it is a blocker —
  the device must not be driven while it is flashing — and the package surface has no input region at all.

A new snapshot replaces the view immediately, so a failure overwrites stale progress rather than leaving it on screen.

### Stacking against the other overlays

Layer rank settles this everywhere except one pairing:

- The firmware surface is on `Top` and registers *before* the alarm (same rank, later registration paints on top), so a
  firing alarm is drawn above the upgrade blocker.
- The package card is on `Bottom`, above the `Background` offline chip — the card temporarily covers the chip instead of
  z-fighting with it in the same corner. This is why it is built by hand rather than with `LayerConfig::bottom_right`,
  which selects `Background`.
- The startup screen is *also* on `Bottom` and registers later, so it would paint over the package card.

That last overlap is accepted rather than fixed, because the two cannot realistically be up together. The startup screen
lives at most ~30 s (up to 20 s waiting for an IP, then a 10 s success or 5 s failure dwell), and nothing puts a package
upgrade inside that window. The recurring check draws its first run 30 minutes to an hour after startup (see
[`../upgrades.md`](../upgrades.md)), so it cannot land during boot. Every other trigger — the one-shot check after
initial device setup, or an upgrade the user starts by hand — comes *from the web UI*, which shows its own progress and
requires the device to already be on the network, which for most of those first 30 s it is not. Ordering the
registrations to give the card priority would trade a case that does not happen for a boot screen the user can no longer
read.

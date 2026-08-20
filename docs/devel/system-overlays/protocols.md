# Overlay Protocols

System overlays use five small Wayland protocols beyond `wlr-layer-shell`, each its own crate at the workspace root
(`deck-screen-edge-v1/`, `deck-settings-v1/`, `deck-device-info-v1/`, `deck-alarm-v1/`, `deck-upgrade-v1/`) beside
`bmc-widget-protocol`. They are shared between the compositor and the overlay framework, so they do not live under
`system-overlays/`. Each crate carries the `.xml` and generates both server and client bindings with `wayland_scanner`
(`generate_server_code!` / `generate_client_code!`), matching the `bmc-widget-protocol` convention.

The first two are forks with deliberately renamed interfaces; the device-info, alarm and upgrade protocols are
Deck-owned. The `deck_` prefix follows the `deck_widget` precedent of not impersonating someone else's protocol: the
contracts differ from their upstreams, so keeping the upstream interface names would mislead the next reader into
assuming upstream semantics. The compositor-side dispatch for all five is in
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

### `deck_settings_v1` (version 3)

Version 2 added a sound-volume slider, a night-mode toggle, a device-restart request, and a `capabilities` bitfield
event (a control whose bit is unset has no backing hardware on the product); see the `.xml` for those members. Version 3
adds the `preempted` event below. The core (v1) shape and the v3 addition:

| Member                  | Kind    | Args                     | Notes                                                                                                                                                                             |
| ----------------------- | ------- | ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `set_brightness(value)` | request | `value: uint` (0–100)    | bmc applies it night-mode-aware and reports the effective value back via `brightness`.                                                                                            |
| `reconfigure_wifi`      | request | —                        | Put the device into WiFi setup mode (open AP + captive portal). One-way; the device leaves setup mode on its own once configured from the phone. Progress via `wifi_ap`.          |
| `destroy`               | request | —                        | Destructor.                                                                                                                                                                       |
| `brightness(value)`     | event   | `value: uint` (0–100)    | Effective brightness. Emitted on bind and on every change, including the night-mode value while night mode is active.                                                             |
| `wifi_ap(ssid)`         | event   | `ssid: string`           | Setup-AP SSID. Non-empty means setup mode is active; empty means inactive. Emitted on bind and on change.                                                                         |
| `preempted(active)`     | event   | `active: uint` (0/1), v3 | A modal full-screen overlay on a layer below the tray (alarm, startup) mapped over the scene (`1`) or cleared (`0`). The tray retracts on `1`. Edge-driven; not replayed on bind. |

**Responsibility split.** The overlay sends `set_brightness` / `reconfigure_wifi` (and the v2 controls); the compositor
forwards them to bmc over the existing action channel and emits `brightness` / `wifi_ap` (and the v2 events) back when
bmc broadcasts. The compositor caches the last brightness and SSID so a late-binding overlay receives current values
immediately on bind. Brightness is applied by bmc night-mode-aware — bmc owns the *effective* value — which is exactly
why the overlay routes through bmc rather than writing the backlight sysfs itself.

`preempted` is different in kind: it does not come from bmc but from the compositor's own view of the layer stack, and
it is *generic* — any full-screen modal overlay drives it, so the tray learns "something took the screen" without
binding that overlay's protocol. Only the settings-tray binds `deck_settings_v1`, so this compositor→tray signal rides
its existing channel rather than a new protocol. The modal-preemption policy (which surfaces count, and the
edge-triggered emit) is in [`compositor-integration.md`](compositor-integration.md).

## `deck_device_info_v1`

New for the device-info overlay. A one-way, event-only state feed: bmc owns the device lifecycle (setup mode, WiFi
provisioning) and its recovery policies, and this interface mirrors that state to the overlay for display. There is
deliberately no control path — everything the user can trigger from these screens is either overlay-local (touch dismiss
of the operational flow) or owned by bmc's own policy timers, so the compositor dispatch is pure fan-out with a replay
cache and needs no command channel back to bmc.

### `deck_device_info_v1` (version 1)

| Member                                     | Kind    | Args                                             | Notes                                                                                                                                                                      |
| ------------------------------------------ | ------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `destroy`                                  | request | —                                                | Destructor.                                                                                                                                                                |
| `device_state(state, boot_flow_delivered)` | event   | `state: uint(enum)`, `boot_flow_delivered: uint` | bmc's `BmcState`; selects the flow. Emitted on bind and on change. See "Once-per-session" below.                                                                           |
| `setup_progress(state, wifi_ssid)`         | event   | `state: uint(enum)`, `wifi_ssid: string`         | Setup-flow transition (`InitSetupState` + an `idle` entry, with the two `unexpected_error` variants split into their own entries). SSID set only for `connecting_to_wifi`. |
| `access_point(ssid, setup_url)`            | event   | `ssid: string`, `setup_url: string`              | Setup-AP SSID and wizard URL, e.g. `http://192.168.8.1/`. Both empty while the AP is down.                                                                                 |

**Replay-on-bind.** Each event's last value is cached compositor-side and replayed on bind, so a late-binding overlay
starts from the complete picture. `device_state` is replayed only once known — an overlay bound before bmc is up keeps
waiting instead of acting on a guessed lifecycle state. `access_point` replays empty strings while the AP is down
(mirroring `wifi_ap`).

`setup_progress` is the one event that is **not** replayed verbatim, because it is the only one describing a
*transition* rather than a current condition. The replay carries only the steps a client cannot reconstruct from the
other events — both `unexpected_error` entries, since nothing else says the device is stuck, and `connecting_to_wifi`,
since mid-join the lifecycle state still reads `factory_default` and its screen advertises an access point the join has
already taken down. Everything else replays as `idle`: a finished setup or reconfiguration is an announcement, and
replaying it makes a client that binds later congratulate the user again long after the fact, while the screens the
remaining steps lead to are reached anyway from `device_state` plus the station address.

**Once-per-session boot screens.** `boot_flow_delivered` is nonzero once an operational lifecycle state has actually
reached a client in this compositor session. The operational connect screens (and the post-upgrade "Update Finished"
screen that opens them) are a boot sequence, not a standing condition — a restarted overlay binds with no memory, so
without this flag every restart would replay the whole sequence over the scenes, and would undo a dismissal that had
already sent it away. It is deliberately a property of the *feed*, not of the receiving client: it latches once and
rides every later `device_state` event, which is what makes it immune to the client-side latest-wins event slots (a
per-bind-only value could be overwritten by a live event arriving in the same dispatch round). Screens reflecting a
standing condition — the whole setup flow — ignore it and are re-derived on every bind. Latching is gated on a live
resource, so a broadcast that reaches nobody (bmc up before the overlay host) does not burn the sequence.

**Responsibility split.** bmc broadcasts through the `Compositor` trait (`broadcast_device_state` /
`broadcast_setup_progress` / `broadcast_access_point`), fed by the device-info listener in `bmc/src/startup.rs`; the
screen-hold timing lives entirely in the overlay — for the transitions it sees. The client slots are latest-wins, one
per event kind (`surface.rs`), so two `setup_progress` transitions landing in a single dispatch round collapse to the
second and the first never gets a screen. That is a real gap in "the overlay owns the hold", not a rounding error, but
it costs a screen only where two steps are microseconds apart: a join that fails on the spot rather than after trying. A
queue would fix it and has not been needed. The `access_point` event deliberately duplicates the settings protocol's
`wifi_ap` (bmc broadcasts once, the compositor fans out to both) so the device-info overlay does not bind
`deck_settings_v1`, which carries tray semantics (preemption, brightness). A retry/reconfigure control on the failure
screens was considered and dropped — recovery lives in the settings tray — and would be a v2 version bump adding a
request plus the inbound command plumbing.

## `deck_alarm_v1`

New for the firing-alarm overlay (`bmc-overlay-alarm`). A lightweight compositor-relayed IPC that shows a ringing alarm
and returns the user's dismiss/snooze. Like `deck_settings_v1` it exists because bmc — not the overlay — owns the alarm
domain (the scheduler, snooze bookkeeping, and audio), so the overlay relays through the compositor's Wayland connection
rather than reaching into bmc directly.

### `deck_alarm_v1` (version 1)

| Member                                       | Kind    | Args                                                             | Notes                                                                                                                                            |
| -------------------------------------------- | ------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `snooze_alarm`                               | request | —                                                                | Ask the compositor to snooze the ringing alarm.                                                                                                  |
| `dismiss_alarm`                              | request | —                                                                | Ask the compositor to stop (dismiss) the ringing alarm.                                                                                          |
| `destroy`                                    | request | —                                                                | Destructor.                                                                                                                                      |
| `alarm_ringing(time, label, snooze_allowed)` | event   | `time: string`, `label: string`, `snooze_allowed: snooze` (enum) | An alarm fired. Carries what the overlay renders: scheduled time (e.g. `07:30`), label (empty if unset), and whether the Snooze button is shown. |
| `alarm_stopped`                              | event   | —                                                                | The alarm stopped for a reason the overlay did not initiate (timeout, dismissal elsewhere, or bmc fallback). The overlay unmaps.                 |

**Responsibility split.** The overlay sends `dismiss_alarm` / `snooze_alarm`; the compositor forwards them to bmc as
lossless `AlarmCommand`s (a dedicated mpsc channel, not the lossy broadcast used for scene events), and bmc's alarm
controller acts. bmc's `AlarmEvent`s are translated the other way into `alarm_ringing` / `alarm_stopped`.
`snooze_allowed` is computed by bmc — `not_allowed` when the alarm has no snooze options or its snooze count has reached
the configured limit — and the limit is *also* enforced in the controller, so the overlay's hidden button is UI, not the
sole guard. The compositor tracks a `ringing` flag and the set of bound overlay resources to drive its no-overlay/crash
fallback ([`compositor-integration.md`](compositor-integration.md)).

## `deck_upgrade_v1`

New for the upgrade-progress overlays (`bmc-overlay-upgrade`). It relays a display projection of bmc's `UpgradeRunState`
to whichever overlays are bound. It is one-way — the overlays never drive an upgrade, so the interface carries no
request beyond the destructor.

Unlike the alarm, this is a *broadcast* protocol rather than a relay for one owning overlay: the two upgrade surfaces
and the startup screen all bind it, and each decides for itself what a snapshot means.

### `deck_upgrade_v1` (version 1)

| Member                            | Kind    | Args                                                | Notes                                                                                        |
| --------------------------------- | ------- | --------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `destroy`                         | request | —                                                   | Destructor.                                                                                  |
| `started(kind)`                   | event   | `kind: kind` (enum)                                 | Opens a snapshot and fixes its presentation kind (`packages` or `firmware`).                 |
| `phase(phase)`                    | event   | `phase: phase` (enum)                               | The current stage. A new phase starts a new progress interval and clears any prior progress. |
| `download_progress(hi, lo)`       | event   | `downloaded_bytes_hi/lo: uint`                      | Bytes downloaded with no known total — an indeterminate bar.                                 |
| `download_progress_with_total(…)` | event   | `downloaded_bytes_hi/lo`, `total_bytes_hi/lo: uint` | Bytes downloaded and the total — a determinate bar.                                          |
| `succeeded(remaining_ms)`         | event   | `remaining_ms: uint`                                | Terminal success, with how long it still has on screen.                                      |
| `failed(remaining_ms)`            | event   | `remaining_ms: uint`                                | Terminal failure, with how long it still has on screen.                                      |
| `snapshot_done`                   | event   | —                                                   | Closes the snapshot; the client commits it.                                                  |

**Snapshot framing.** Wayland dispatch batches events, so a client that acted on each event as it arrived could paint a
phase from one snapshot with a byte count from the next. Every snapshot is therefore bracketed by `started` and
`snapshot_done`, and a client ignores anything outside that bracket and commits only on `snapshot_done`. A sequence with
an invalid enum value or bad ordering is discarded whole and the client keeps its last coherent view.

**Byte counts.** Wayland has no 64-bit integer argument, so each byte count is split into two `uint` words (`_hi` /
`_lo`) rather than truncated to 32 bits.

**Responsibility split.** bmc owns every upgrade decision; the protocol carries presentation only. `remaining_ms` is one
consequence: how long a finished upgrade stays on screen is decided by bmc's display projection, and the compositor
recomputes the remainder against the cached deadline on each replay so a client binding late gets the time actually
left, not a fresh full interval.

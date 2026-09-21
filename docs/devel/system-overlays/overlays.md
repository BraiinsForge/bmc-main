# The Concrete Overlays

Five overlays ship today, each a small crate under `system-overlays/` implementing `SystemOverlay` (see
[`framework.md`](framework.md)). This document covers what each shows, when it maps and dismisses, where its data comes
from, and its platform gating.

## Device info (`bmc-overlay-device-info`)

The full-screen transient boot and setup screens: the first-boot setup flow (AP SSID + QR, connecting, connected,
device-setup IP QR, completed, errors), the same flow over an ethernet cable on the boards that have a port, WiFi
reconfiguration, and the operational-boot connect-info sequence. It is a port of the stable-26.02 `display_tasks`
screens onto the overlay framework.

`LayerConfig::fullscreen` with the layer lowered to `Layer::Bottom`, full input region. It blocks scene touch while
shown, but sits below a firing alarm (`Top`), the firmware-upgrade splash (`Top`), and the settings tray (`Overlay`).
Because it is a full-screen surface above `Background`, the compositor's `is_fullscreen_blocker` predicate suppresses
scene swipes for as long as any screen is up — the scenes handoff is purely the unmap (see
[`compositor-integration.md`](compositor-integration.md)).

### Inputs

bmc owns the lifecycle and drives the overlay over `deck_device_info_v1` (see [`protocols.md`](protocols.md)):
`device_state` selects the flow, `setup_progress` steps the setup flow, and `access_point` carries the setup-AP SSID and
wizard URL (so the overlay hard-codes no AP addressing). The displayed device address comes from the connectivity
prober's `station_ipv4` — the pick that excludes AP-mode interfaces, so the setup AP's own address never counts as an
uplink. Until the first `device_state` event the overlay stays unmapped rather than guess a flow.

`access_point` says one of three things: the SSID and wizard URL while the setup AP is up, the URL alone (an empty SSID)
where the wizard is reached over the cable, and two empty strings while neither is reachable, which is the pending
screen. bmc refreshes it every two seconds for as long as a setup screen is up (`publish_access_point` in
`bmc/src/startup.rs`). The cable wins while the port holds a routable address and its link is up; the SSID is advertised
only while the AP is up, which bmc asks of whatever owns the AP on that board — its own provisioning state where it
raises the AP itself, and whether `network.wifi_ap.ipaddr` is bound to an interface on the BMM boards, where the
platform's hotplug parks the AP behind bmc's back once the cable holds an address and starts it again when the cable
goes; and a destination that stays unreachable for three refreshes is cleared rather than left up as a dead URL. Nothing
reachable for half a minute on a board with WiFi is the AP failing to come up, reported and, mid-setup, followed by a
reboot. A board without WiFi has no AP to bring back, so it keeps waiting for the cable instead.

What the board can offer at all, WiFi, an ethernet port, or both, the compositor tells the overlay over
`deck_platform_v1` (`wifi` and `ethernet` bits of `capabilities`), together with the product name the screens address
the user with (`product_name`) and whether the board mines (`mining`, which picks the device artwork). The overlay opts
in with `uses_platform` and never reads the hardware profile itself. The uplinks decide the wording of every screen that
waits on a connection: a WiFi join once the board has WiFi and a network to name (the target from `connecting_to_wifi`,
or the saved station network), else the cable. Until the compositor has said what the board has, it reads as the Deck.

A fourth event, `report_ip`, is the IP-report button reaching the overlay: it raises the operational connect-info screen
on demand, on the same timer a boot uses. See "IP-report button" below.

The three state events replay on bind, but the overlay is careful about what a replay is allowed to start, because a
restarted overlay binds with no memory of what it already showed. The **setup** screens reflect a standing condition —
the device really is waiting in setup right now — so they are re-derived on every bind, and the setup connect-info comes
back on its own from `device_state` plus the station address. The **operational connect** screens are a boot sequence
instead: they run once per session, gated on `device_state`'s `boot_flow_delivered` flag, so a restart neither replays
them over the scenes nor undoes a dismissal. On the setup side the same distinction is drawn compositor-side, by not
replaying the announcement steps.

Every screen-hold timer lives in the overlay; bmc emits transitions the moment they happen (recovery policies — the
no-IP factory reset, and a reboot when the setup AP will not come up on a device that has nothing to fall back to — stay
in bmc, which broadcasts the failure before acting, flagged as restarting so the screen only promises a restart that is
actually coming; the same failure during WiFi reconfiguration leaves the running device alone).

### Flows

- **Operational boot**: connecting (SSID, "waiting for IP") → connect-info (IP + QR, 10 s) on an address, or failure (5
  s) after 20 s without one → unmap. A touch dismisses this flow immediately; a post-upgrade boot skips it or opens on
  the upgrade-success screen (see "Opening after an upgrade" below — that applies to this flow only, never the setup
  screens). A board waiting on its cable says "Connecting to network..." and "No network connection" with "Check the
  Ethernet cable" instead, and never mentions WiFi.
- **First boot** (`factory_default`): setup-start (AP SSID + QR of the wizard URL; a placeholder until `access_point`
  arrives) → `connecting_to_wifi` → connected (5 s) → setup connect-info (device-setup IP QR) → `device_setup_success` →
  completed (5 s) → unmap. A `wifi_connection_failed` shows the error until the setup AP (or the cable) is announced
  again, then returns to setup-start, and the join target is forgotten with it. Setup screens ignore touch — dismissing
  them would hide the wizard with the AP still up.
- **First boot over a cable** (`factory_default` with the cable in, or plugged in during the AP screen): setup-start
  shows the wizard address and its QR alone, since there is no AP to join, and a board with both uplinks offers the
  cable beside the SSID while the AP is up. Skipping WiFi in the wizard raises the switchover screen ("Your device is
  being set up...") until the lifecycle advances, which lands on the setup connect-info at once where the cable's
  address is already known. A cable pulled during the AP-pending screen brings the SSID back once hotplug has the AP up;
  pulled while the setup connect-info is up, it returns the screen to the connect progress after `ADDRESS_LOSS_GRACE`
  (10 s), and the address comes back the way it first arrived. A cable pulled once the lifecycle has advanced to
  SetupPending brings no AP back, since the platform's hotplug only acts while the board is factory-default, so the
  connect progress is the end state until the cable returns. A board without WiFi asks for the cable in place of the AP.
- **SetupPending boot** (configured but unfinished): connecting, self-advancing to the setup connect-info when the
  station address appears; bmc's watchdog factory-resets if none comes, but only when every poll could actually read the
  uplink — a failed read is not evidence, and the reset destroys the configuration. Where the board has an ethernet port
  and no station network is configured there is no join to judge, so bmc runs no watchdog and the screen waits on the
  cable.
- **WiFi reconfiguration**: the same setup flow, entered when `device_state` flips to `wifi_reconfiguration`; on
  `wifi_reconfig_success` the connected screen shows 5 s, and on an operational device unmaps straight to scenes (no
  connect-info). The setup-start screen holds like a first boot's, for as long as the AP is up: the user asked for this
  flow from the tray, and hiding it would leave the AP broadcasting behind the scenes with nothing on screen to say so.
  Entering the flow replaces whatever setup screen was up, so a device parked on the connect-info with its setup
  unfinished shows the AP the moment the tray starts it. Its setup is still unfinished: the join clears only the
  reconfiguration flag and the lifecycle drops back to `setup_pending`, so the connected screen goes on to the
  connect-info instead (`Mode::setup_done` decides) and the wizard finishes at the new address.
- **Unexpected error**: a full-screen failure, in two variants that differ in what happens next rather than in how bad
  the failure is. `unexpected_error_restarting` means bmc is restarting or resetting the device, so the screen says so,
  waits it out, and ignores touch — the restart is coming and there is nothing to dismiss it *to*. `unexpected_error`
  means bmc takes no action, so the screen asks the user to restart it instead. That one steps aside after
  `FATAL_SCREEN_TIMEOUT` (1 min) or on a touch, but **only once the setup is done** (`Mode::setup_done`: a
  reconfiguration or an operational device). Mid-setup it stays put rather than hide the unfinished wizard behind the
  scenes. Dismissing it does not hide the underlying condition: a setup AP still broadcasting after a failed
  reconfiguration exit shows on the settings-tray button, which reads `wifi_ap` from `deck_settings_v1`.

Both connect-info screens hold the last-known address through a transient DHCP loss rather than reading the prober live.
On the operational screen that stops a flicker; on the setup screen it matters more, since falling back to the
connect-progress screen would be a dead end — no bmc event is coming, there is no deadline, and setup screens ignore
touch — while the address is exactly what the user still needs to finish the wizard in a browser. An address reached
over the cable is the exception on the setup screen: after `ADDRESS_LOSS_GRACE` without it the screen does fall back,
because a pulled cable is the likely cause, the connect progress brings the address back on the next lease, and a dead
wizard URL helps nobody. Which it is, the overlay reads off the wording the connect progress would use, so a board that
has an ethernet port but joined Wi-Fi keeps its address like the Deck: a station that drops out comes back with the same
one. The screens render through the `bmc-render` tree pipeline with the legacy init-setup SVG icons, the miner outline
and the ethernet icons embedded at build time; every screen has a gallery cell, one scene per product
(`Overlays / Device Info / BMC100 | BMM101 | BMM100` in `overlays.scene.rs`) staging the screens that product can show
at its own panel, and the `Screen` knob picks one card for `capture.toml`.

### IP-report button

A `report_ip` event puts the device address back on screen after the boot sequence is long gone: the overlay raises the
operational connect-info screen for its usual 10 s, or the failure screen where the device has no address, since a press
deserves an answer either way.

bmc sends it on a short press of the IP-report button (`ButtonId::IpReport`, `bmc/src/button_manager.rs`). The button
arrives from the kernel as `BTN_0` and is handled wherever the kernel reports it; BMM100 and BMM101 are the products
wired with one today. A release strictly under `BOSER_REPORT_IP_MAX_HOLD_DURATION` (1 s, boser's bound for sending the
IP-report packet) becomes `broadcast_report_ip`.

A hold that reaches `DISPLAY_OFF_MIN_HOLD_DURATION` (3 s) turns the display off the moment it gets there, while the
button is still down, and never raises the address. The button loop arms a deadline on the press and acts on it ahead of
the event stream, so a release landing in the same instant loses to the blank. Boser does nothing past its own 1 s
bound, so the 3 s bound is the BMC application's alone. A release between the two bounds does nothing.

Both outcomes travel on one `watch<ScreenRequest>` that `bmc/src/system_manager.rs` defines. Every handled press and
every touch writes `ScreenRequest::Wake`; the hold overwrites it with `ScreenRequest::Blank` at the bound. Releases
write nothing. `run_screen_auto_off` reads the current value at the top of each iteration and reconciles the panel
toward it, which is why a request needs no acknowledgement and why a touch landing mid-blank is not lost: it can only be
superseded by a later writer, never miss a reader that was busy. The blank holds until the next write of `Wake` rather
than until a timeout, so the loop sits in `AutoOffMode::HoldDark` meanwhile. A ringing alarm refuses the request and
overwrites it back to `Wake`, so it is not replayed when the ring stops; see [Night Mode](../../stories/night-mode.md).
The blank resets the cycler to the first scene, but only night mode suspends cycling, so outside it the compositor keeps
rendering scene transitions to a dark panel and the wake shows whatever scene cycling has reached.

The address comes from the same connectivity prober the boot screens read. Its thread keeps publishing while the overlay
is unmapped, and a publish that changed the content moves the snapshot version, so the poll on the press picks up
whatever changed while nothing was watching.

Three conditions gate the press: the device must be operational, no setup flow may be live on screen, and a boot must
not still be on its connect or post-upgrade screen. A setup screen hands back to the scenes when it expires and a device
mid-setup has none to hand back to, and reconfiguration reaches `operational` before its closing setup event, which is
why the lifecycle state alone does not decide it. A fatal screen the user can dismiss, the one drawn with the close
glyph, is not a live flow: a press replaces it the way a touch would, and shows the address where the touch shows
nothing. A fatal waiting for a restart stays, as it does for a touch. A boot's connect screen is about to show the
address on its own, and a press before the lease arrives would replace the wait with a failure screen the lease could
never undo. `boot_flow_delivered` is deliberately ignored: it exists so a restarted overlay does not replay a *boot*
sequence, and a button press is not a boot.

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

The snapshot's `remaining` dwell is ignored: like every other screen here, this one is timed by the overlay. And because
the screen only ever opens the operational flow, `boot_flow_delivered` covers it too — a restarted overlay does not
confirm an upgrade the user was already told about.

Which of the two paths runs is decided by the runner, not by the wire: the device-info events are drained before the
snapshot is applied in `tick`, whichever order the compositor replayed them in. So on a post-upgrade boot the
`device_state` has already opened the connect screen, and the snapshot switches it to the upgrade screen on the spot,
gated on `Connecting` so an upgrade finishing minutes later cannot resurrect it. The *latch* covers the other order,
where the snapshot lands with no connect screen to replace: only the operational entry consumes it, so a success
arriving mid-setup cannot disturb the setup screens.

## Offline and mining status (`bmc-overlay-offline`)

The passive bottom-right corner. Two indicators share its one surface, and the crate keeps the name of the first:

- the **"OFFLINE" chip**, mapped while the device has no routable IPv4 and unmapped again when connectivity returns;
- the **mining-status pickaxe**, a 20×20 icon centred on a square translucent card of the chip's height, violet while
  the miner tunes and red while it underperforms, is stopped, or cannot be reached. A miner that is simply mining draws
  nothing: there is no positive indicator, so an empty corner is the healthy state.

`LayerConfig::bottom_right("bmc-overlay-offline", (160, 48))` → `Layer::Background` with **no input region**, so touches
in its corner fall through to whatever is behind it. `Background` is the lowest rank, so every other overlay draws over
it; it still paints above the scene, so the indicator stays visible over the clock.

### The chip

`tick` polls the prober's `snapshot_if_changed` on every wake (`POLL` = 2s) and keeps the state derived from the last
changed snapshot; the chip shows exactly when a published snapshot holds no routable IPv4 (before the first snapshot it
stays hidden, so boot never flashes it). It is a content-tight card at the bottom-right corner (translucent black
background, red label), with the rest of the surface transparent. The pickaxe's card shares its height and background,
so the two read as one indicator changing content.

### The pickaxe

It exists only where the compositor's `deck_platform_v1` reports the `mining` capability (BMM100, BMM101, BFM100). The
overlay opts into that protocol (`uses_platform`), and `on_platform_capabilities` starts the polling thread when the bit
is set; on a Deck the thread never starts and the corner is the chip alone. The gate is inside the overlay rather than
in the host's registry because the surface has to exist everywhere for the chip.

The thread (`poller.rs`) reads the local boser's REST API every 5 s with a 1 s request cap, over the login mechanism the
miner-info widget also uses: a token from `POST /api/v1/auth/login`, sent bare in the `Authorization` header. The widget
asks the user for the BOS password and polls nothing until it has one; the overlay has nobody to ask and logs in as
`root`/`root`. (A user who changed the BOS password gets 401s here and a red pickaxe, until boser issues on-device
clients a local token file and the constant goes. A refused login is retried on the widget's doubling delay, 10 s up to
5 min, so the polls keep counting against the budget without hitting boser's auth log every 5 s.)
`GET /api/v1/performance/tuner-state` and `GET /api/v1/miner/hw/hashboards` are the only two reads (`bos.rs`); bosminer
IPC and gRPC are not involved. `BMC_MINING_API_URL` points the poller at a `bmc-netsim` instance during development
(`just netsim::run mining-status` serves one miner per state on pinned ports).

The status rule (`mining.rs`) reads the tuner state and the hashboards, and nothing else:

- a board is *active* when `enabled` and its nominal is above 0, *hashing* when active with `last_1m > 0`, and
  *underperforming* when active with `last_5m` null or under 80 % of nominal;
- **Tuning** (violet) when `overall_tuner_state` is TUNING, CONTINUOUS or PREHEAT and at least one board hashes;
- **Ok** (nothing) when no tuning stage is reported, at least one board is active and none underperforms;
- **Low** (red) otherwise, including a tuning stage with no board hashing, which is how a miner paused mid-tune reads
  once its 1-minute rate has drained. Disabled boards are ignored, so one left off never holds the corner red.

Any failed poll — no answer, a login refusal, a non-2xx read (boser sends 412 while bosminer is down), a body that does
not parse — keeps the last answer for 5 consecutive failures and then shows Low. Before the first success there is
nothing to keep, so the pickaxe stays hidden until the budget runs out. A success resets the budget at once. The failure
that uses up the budget is logged once at `warn` with its cause, and the success that ends the outage once at `info`;
the polls in between stay at `debug`.

A pause, a dead pool or a stopped process reaches the corner through the hashrate means, about a minute late, rather
than from a liveness read. That trade keeps the overlay at two REST calls; the lifecycle and pools endpoints exist if a
faster answer is ever wanted.

### The corner

`decide` folds the two into one `OfflineView`: the chip takes the corner outright, and the pickaxe is drawn only while
the device is online and the status is Tuning or Low. Polling continues while offline, so the pickaxe returns in its
current state with connectivity. A frame is requested only when the visible view changes, so a chip-to-pickaxe or
violet-to-red flip repaints and an unchanged corner does not. The gallery's `Offline` scene drives every combination
from the same `decide`, on the bare surface and on a 480×320 BMM101 backdrop.

## Settings tray (`bmc-overlay-settings-tray`)

The swipe-from-top quick-settings panel: ± brightness and volume controls, a night-mode toggle, and hold-to-confirm
restart and WiFi reconfigure buttons over the WiFi station info. It is the only overlay that uses both vendored
protocols. It is ported from the BDK-343 `settings-stub` widget, translated to the native `bmc-render` tree.

Its `LayerConfig` is built by hand: `Layer::Overlay`, anchored to all four edges, **full** input region (the tray is
full-screen and blocks scene swipes while it is up). `screen_edge()` returns `ScreenEdge::Top` and `uses_settings()` is
`true`.

### Reveal and dismiss

The panel is armed to the top edge: hidden (no buffer) until the compositor's top-edge swipe reveals it. On reveal
(`on_reveal`) it resets its FSM and touch tracking and starts the slide. The hosted slide moves the attached panel
through layer-shell margins: paint once on opening, then reuse the content unless it changes. No separate panel-image
cache is retained (see [`framework.md`](framework.md)); `SLIDE_MS = 180` ms, eased.

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

|           | Firmware                                  | Packages                                      |
| --------- | ----------------------------------------- | --------------------------------------------- |
| Placement | full-screen (`LayerConfig::fullscreen`)   | bottom-right card, or full-screen — see below |
| Layer     | `Top`                                     | `Bottom`                                      |
| Input     | full                                      | none                                          |
| Effect    | modal: blocks the scene for the whole run | passive: widgets stay visible and interactive |

Both clients bind the protocol and receive every snapshot; each maps only for its own kind and clears its view when a
snapshot of the *other* kind arrives. The inactive client stays unmapped and holds no DMA-BUFs, so the split costs
nothing while idle. A firmware-containing run uses the firmware surface even when it also carries packages — anything
that reboots the device blocks the screen.

Making one surface reconfigure its size, anchors, layer, and input policy at runtime was the alternative. It was
rejected: it would add runtime surface reconfiguration to the overlay framework for a single caller.

### Package surfaces

The firmware surface is full-screen everywhere and takes whatever size the compositor configures. The package surface
comes from `SurfaceTier::for_package_display(display)`, keyed on the display width, because `LayerConfig` is read before
any size exists:

| Display width | Package surface                                                         | Displays         |
| ------------- | ----------------------------------------------------------------------- | ---------------- |
| ≥ 960         | 384×192 card, bottom-right                                              | 1280×480         |
| ≥ 400         | 240×120 card, bottom-right                                              | 480×320, 480×480 |
| below         | full-screen: a card holding the content would cover most of the display | 320×240          |

Width alone decides it: the card is wider than it is tall, and no display is taller than it is wide. On the round
480×480 panel a corner card is wrong by construction whichever size it takes, and the BFM100 has no upgrade design of
its own yet; the smaller card at least loses less of itself over the edge. Full-screen is not the way out there, because
that panel has touch — see the blocker note below.

A full-screen package surface stays on `Bottom` with no input region, so it never takes touch. It does, however, satisfy
the compositor's `is_fullscreen_blocker` test, which is purely geometric — any mapped non-`Background` surface covering
the output — so while it is up, scene-drag is suppressed and the settings tray is retracted (see
[`compositor-integration.md`](compositor-integration.md)). On the BMM100 that costs nothing, because the board has no
touchscreen to drag a scene or open the tray with. On a product with touch, the same surface would make a package
upgrade behave modally after all; `Layer::Background` would be the way out, at the price of the offline chip painting
over the screen instead of being covered by it.

### Sizing tiers

`build_upgrade_tree` takes a `Surface` — a size plus the `SurfaceTier` it is — and reads its numbers from a `Tier` that
`tier_for` looks up, one entry per variant. Type and icons do not scale linearly with a display, so each tier states its
own numbers instead of deriving them from a factor.

| Tier          | Serves                         | Of note                                                  |
| ------------- | ------------------------------ | -------------------------------------------------------- |
| `FULL_LARGE`  | BMC100 1280×480                | the stable Deck screen                                   |
| `FULL_MEDIUM` | BMM101 480×320                 | Deck type and icon; only the bar scales with the display |
| `FULL_SMALL`  | BMM100 320×240, **both kinds** | smaller type, and the safety text wraps onto two lines   |
| `CARD_LARGE`  | BMC100 384×192                 | the stable Deck card                                     |
| `CARD_SMALL`  | BMM101 240×120                 | 18px type, no byte counts, `Downloading 54%...`          |

The icon's top edge is one of those numbers, fixed per tier, and everything else is laid out downwards from it.

Every width threshold lives on `SurfaceTier` in `lib.rs`, so nothing downstream re-derives a tier from a size: adding a
surface means adding a variant, and the compiler then points at each match that has not handled it. The two cuts there
ask different questions — `for_package_display` whether a card leaves its widget visible, `fullscreen_for_width` whether
the type stays legible — and reach the same answer on every display that exists only because both sets of thresholds sit
in the gaps between 320, 480 and 1280. Neither is derived from the other, and they are free to diverge.

Two content decisions belong to the tier rather than to the kind, because narrow surfaces drive them: whether the
determinate screen draws the transferred/total byte counts, and whether its caption drops the subject noun
(`UpgradePhase::short_label`). Everything else keyed on presentation stays keyed on the upgrade kind — the safety text
is a firmware screen's, and the activity bar under a phase with no byte totals is a package screen's.

The two edge dividers key off `SurfaceTier::is_card`, not the kind: they exist to give a card an extent against the
black widgets it overlaps, and a full-screen package surface has nothing beside it.

A canvas text draw is always a single unwrapped line — `max_width` and `text_overflow` never reach the layout — so the
one tier that needs wrapping draws the safety text through `DrawCommand::AutofitText`, whose paragraph path takes a box.
`min_size` equal to the style size makes it wrap without shrinking. The wider tiers stay on the single-line draw: the
two paths anchor differently (glyph centre against line box), so moving stable copy across would shift it by a few
pixels for no gain.

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

# BMC Feature Stories

User-facing feature documentation for the Braiins Deck. Each document captures user stories, behavior, and constraints
for a single feature area.

## Features

### [LED Notifications](led-notifications.md)

A strip of 10 addressable LEDs provides ambient status feedback — device boot, Wi-Fi connectivity, firmware upgrades,
alarms, and price movements — without requiring the user to look at the screen. Effects are prioritized so concurrent
events resolve deterministically, and a master toggle lets the user silence them entirely.

### [Widget LED Effects](widget-led-effects.md)

Widgets can drive the same LED strip. Effects are scene-scoped by default — only visible while the widget is on screen —
and widgets can opt into ambient effects that follow the user across scenes. System events always take priority so
device-level alerts are never buried by a widget animation.

### [Support Archive](support-archive.md)

A one-click diagnostic bundle downloadable from the web UI. Collects system state, logs, network diagnostics, and Nix
package state (profile generation manifests, profile state summary, Nix database) into a single timestamped file the
user can attach to a support request. Collection is best-effort — one unavailable source never blocks the archive.

### [Support Archive Credential Censoring](support-archive-credential-censoring.md)

The support archive automatically censors credentials (Braiins Pool API keys, Wi-Fi passwords) before bundling
diagnostic files, so users can safely share archives with support without exposing secrets.

### [Scenes & Widgets](scenes-and-widgets.md)

Scenes are user-configured display pages containing fullscreen or combined widget layouts. Users can enable, disable,
reorder, swipe between, and automatically cycle through scenes while keeping widget settings scoped to each widget
instance. A scene with no rendered content yet shows a loading placeholder.

### [Combined Scene](combined-scene.md)

A combined scene lets the user compose multiple compatible widgets on a single display. Widgets share a common sizing
model (`small`, `medium`, `large`) and keep their own widget-specific configuration while rendering as one scene, laid
out on a uniform separator grid. `full`-sized widgets are reserved for fullscreen scenes.

### [Default Scenes](default-scenes.md)

Factory default scene sets matched to each product, so a freshly provisioned or factory-reset device shows relevant
content out of the box. Defaults cycle automatically and apply only when the device has no usable configuration.

### [Nix Store & Profile Power-Loss Safety](nix-store-durability.md)

Durability guarantees for the on-device software store: losing power during first-time installation, an upgrade, or a
recovery wipe never leaves the device trusting corrupt or incomplete software. Interrupted installs restart cleanly,
interrupted upgrades fall back to the previous working version, and storage errors fail loudly instead of masquerading
as success.

### [Nix Store Initialization](nix-store-initialization.md)

How a device upgrading from firmware without Nix support gains its package store: the ordinary firmware upgrade
downloads the store contents published for exactly that firmware release and verifies their Ed25519 signature against a
factory-provisioned key before installing anything, so a network attacker cannot plant tampered software. Inconsistent
stores are wiped and reinitialized at the next firmware upgrade.

### [Touch & Gestures](touch-and-gestures.md)

Swipe left/right to navigate between scenes and tap/drag to interact with widgets via touch events forwarded through the
Wayland protocol.

### [Settings Tray](settings-tray.md)

A persistent overlay, revealed by swiping down from the top edge of any scene, for quick access to core system settings:
brightness, sound volume, night mode, hold-to-confirm device restart, and WiFi reconfiguration. Controls are gated by
what the device actually supports.

### [Night Mode](night-mode.md)

Scheduled quiet-hours behavior that uses separate brightness and sound volume, optional screen auto-off with touch and
button wakeup, and separate LED notification enablement for night hours.

### [Web UI for Deck Configuration](web-ui.md)

Browser-based interface hosted by the device for managing scenes, widgets, and system-wide settings. Widget catalog and
config forms are manifest-driven — any installed widget (including out-of-tree) is configurable without a firmware
rebuild. The catalog groups widgets into labeled category sections (with "Other" last) and offers category filter pills
with per-category counts.

### [Widget Installation](widget-installation.md)

The backend can discover available-but-not-installed widget packages and install them through the shared upgrade flow.
The frontend picker integration and interactive installation experience will be delivered in a future change.

### [Seamless Widget Upgrades](seamless-widget-upgrades.md)

Widget-only upgrades replace changed widget instances without blanking or restarting the whole display. Host, thin, or
compositor changes still converge safely through service dependencies and compositor-owned process identities.

### [Credential Accounts](credential-accounts.md)

Saved accounts hold a service credential — a Braiins Pool token, an API key, a username and password — that the user
enters once and binds to any widget needing one. The widget sees only that a credential is available and which account
it came from; the device attaches the secret to the widget's outgoing requests itself, and refuses to send a
service-tied credential anywhere but its own service.

### [Widgets](widgets/README.md)

Information about all implemented official widgets.

### [Config Migration on Firmware Upgrade](config-migration.md)

When upgrading from the slint-monolith firmware to the manifest-driven widget system, the existing config is converted
automatically. Scenes and widget positions survive; widgets this firmware can't translate are dropped with a warning
rather than kept as placeholders.

### [Upgrade Progress](upgrade-progress.md)

On-device feedback for firmware and package upgrades. A firmware upgrade takes over the screen with a modal full-screen
progress overlay; a package-only upgrade shows a passive corner card while widgets keep running. Both end in a clear
success or failure screen, including after the restart that finishes an upgrade.

### [Device Setup & Connect Screens](device-setup-screens.md)

The full-screen messages the Deck shows on its own display when it needs setting up or has just booted: the WiFi network
to join and a QR code to the setup wizard on a factory-default device, the progress of a WiFi join, and the address the
web UI is reachable at after every boot. Also covers re-running WiFi setup from the device and the confirmation shown
after a firmware update restarts.

### Audio & Alarms

See [Clock Alarm](alarm.md) for the whole alarm feature — configuring alarms from the web app (time, repeat days, label,
sound, and per-alarm snooze) and the on-screen behavior of a firing alarm (the ringing screen, Stop/Snooze, and snooze
limits).

### Display & Scenes

*Partially documented.* See [Night Mode](night-mode.md) for scheduled quiet-hours behavior and
[Scenes & Widgets](scenes-and-widgets.md) for scene management and cycling behavior.

### Network Management

*Not yet documented.* Ethernet (DHCP / static IP) and Wi-Fi configuration, network scanning, and saved-network
management.

### [Software Upgrades](software-upgrades.md)

Firmware and application updates share one over-the-air upgrade flow. Automatic updates check throughout the day, so a
device can stay current even when it is normally offline at night, while per-device staggering spreads service load. See
[Upgrade Progress](upgrade-progress.md) for the on-device progress UI.

### Authentication & Accounts

*Not yet documented.* User login, password management, multi-account support, and app integration.

### Price Alerts

*Not yet documented.* Cryptocurrency price monitoring with configurable notifications and LED feedback.

# BMC Feature Stories

User-facing feature documentation for the Braiins Deck. Each document captures user stories, behavior, and constraints
for a single feature area.

## Features

### [LED Notifications](led-notifications.md)

A strip of 10 addressable LEDs provides ambient status feedback — device boot, Wi-Fi connectivity, firmware upgrades,
alarms, and price movements — without requiring the user to look at the screen. Effects are prioritized so concurrent
events resolve deterministically, and a master toggle lets the user silence them entirely.

### [Support Archive Credential Censoring](support-archive-credential-censoring.md)

The support archive automatically censors credentials (Braiins Pool API keys, Wi-Fi passwords) before bundling
diagnostic files, so users can safely share archives with support without exposing secrets.

### [Scenes & Widgets](scenes-and-widgets.md)

Scenes are user-configured display pages containing fullscreen or combined widget layouts. Users can enable, disable,
reorder, swipe between, and automatically cycle through scenes while keeping widget settings scoped to each widget
instance.

### [Combined Scene](combined-scene.md)

A combined scene lets the user compose multiple compatible widgets on a single display. Widgets share a common sizing
model (`small`, `medium`, `large`) and keep their own widget-specific configuration while rendering as one scene.
`full`-sized widgets are reserved for fullscreen scenes.

### [Nix Store Initializer](nix-store-initializer.md)

A last-resort recovery component that runs on every boot. When the device has no initialized Nix store — typically after
a factory reset — it takes over the display, guides the user through WiFi setup via an open access point, downloads and
activates the Nix store, and falls back to firmware upgrade if no matching bundle exists.

### [Touch & Gestures](touch-and-gestures.md)

Swipe left/right to navigate between scenes and tap/drag to interact with widgets via touch events forwarded through the
Wayland protocol.

### [Night Mode](night-mode.md)

Scheduled quiet-hours behavior that uses separate brightness and sound volume, optional screen auto-off with touch and
button wakeup, and separate LED notification enablement for night hours.

### [Web UI for Deck Configuration](web-ui.md)

Browser-based interface hosted by the device for managing scenes, widgets, and system-wide settings. Widget catalog and
config forms are manifest-driven — any installed widget (including out-of-tree) is configurable without a firmware
rebuild.

### [Widgets](widgets/README.md)

Information about all implemented official widgets.

### Audio & Alarms

*Not yet documented.* Alarm scheduling with repeat patterns, snooze, and custom sound playback through the on-board
speaker.

### Display & Scenes

*Partially documented.* See [Night Mode](night-mode.md) for scheduled quiet-hours behavior and
[Scenes & Widgets](scenes-and-widgets.md) for scene management and cycling behavior.

### Network Management

*Not yet documented.* Ethernet (DHCP / static IP) and Wi-Fi configuration, network scanning, and saved-network
management.

### Firmware Upgrade

*Not yet documented.* Over-the-air firmware download, installation, and auto-upgrade scheduling (daily, weekly,
bi-weekly, monthly).

### Authentication & Accounts

*Not yet documented.* User login, password management, multi-account support, and app integration.

### Price Alerts

*Not yet documented.* Cryptocurrency price monitoring with configurable notifications and LED feedback.

# BMC Feature Stories

User-facing feature documentation for the Braiins Deck.
Each document captures user stories, behavior, and constraints for a single
feature area.

## Features

### [LED Notifications](led-notifications.md)

A strip of 10 addressable LEDs provides ambient status feedback — device boot,
Wi-Fi connectivity, firmware upgrades, alarms, and price movements — without
requiring the user to look at the screen.  Effects are prioritized so
concurrent events resolve deterministically, and a master toggle lets the user
silence them entirely.

### [Support Archive Credential Censoring](support-archive-credential-censoring.md)

The support archive automatically censors credentials (Braiins Pool API keys,
Wi-Fi passwords) before bundling diagnostic files, so users can safely share
archives with support without exposing secrets.

### Audio & Alarms

*Not yet documented.*  Alarm scheduling with repeat patterns, snooze, and
custom sound playback through the on-board speaker.

### Display & Scenes

*Not yet documented.*  Screen brightness, night mode scheduling, and scene
management for fullscreen and combined content layouts with configurable cycle
durations.

### Network Management

*Not yet documented.*  Ethernet (DHCP / static IP) and Wi-Fi configuration,
network scanning, and saved-network management.

### Firmware Upgrade

*Not yet documented.*  Over-the-air firmware download, installation, and
auto-upgrade scheduling (daily, weekly, bi-weekly, monthly).

### Authentication & Accounts

*Not yet documented.*  User login, password management, multi-account support,
and app integration.

### Price Alerts

*Not yet documented.*  Cryptocurrency price monitoring with configurable
notifications and LED feedback.

# Braiins Deck — Architecture Overview

Braiins Deck is a standalone Bitcoin dashboard display. It shows real-time market data, mining stats, and customizable
widgets on a 1280x480 touchscreen. The hardware is an STM32MP157C (dual Cortex-A7) running OpenWrt Linux.

## Repositories

| Repo         | What                                        | Where                        |
| ------------ | ------------------------------------------- | ---------------------------- |
| **bmc-main** | Deck application firmware (this repo)       | gitlab / forge/deck/bmc-main |
| **bos-main** | BraiinsOS mining firmware (bosminer, boser) | gitlab / bos/bos-main        |
| **openwrt**  | Braiins OpenWrt fork with board support     | gitlab / bos/openwrt         |

bmc-main and bos-main share some libraries (`bmc-net-types`, `bmc-net-drv`, `time`, etc.) via the `bmc-net/` and
`bmc-shared/` workspace members, the latter mirroring `open/utils-rs/` from bos-main.

## Key Crates

### Platform layer

- **bmc-openwrt** — the main binary. Platform glue: DRM rendering, WiFi detection, backlight, LED driver, touch input,
  CLI. Cross-compiled as a static musl binary for ARM (production) and x86_64 (QEMU emulation).

### Core logic

- **bmc** — state machine, startup orchestration, web server, gRPC API; also owns the persisted scene-cycling and
  account data structures (rendering itself lives in the per-widget crates under `widgets/`).

### Hardware drivers

- **bmc-led** — APA102 addressable LED strip over SPI
- **bmc-button** — hardware button via uevent
- **bmc-gpio** — GPIO via sysfs/gpiod
- **bmc-kobject** — kernel uevent listener

### Shared

- **bmc-net/bmc-net** — the `NetworkManager` facade: network config, provisioning state machine, setup AP and captive
  portal, with `openwrt` (UCI), `buildroot` and `mock` backends
- **bmc-net/bmc-net-types** — dependency-light value types (`MacAddr`, network protocol config, WiFi status/scan)
- **bmc-net/bmc-net-drv** — interface enumeration plus the `WifiDriver` backends: `nl80211` (OpenWrt UCI/ubus) and
  `esp32` (ESP32-over-SDIO setup AP)
- **bmc-net/bmc-net-dns** — the `IiResolver` DNS/NTP resolver
- **bmc-net/bmc-net-mdns** — mDNS/DNS-SD advertisement of the device web UI and API
- **bmc-net/bmc-net-observe** — synchronous, read-only connectivity probes for OS-driven overlays
- **bmc-net/bmc-net-diag** — network diagnostics for the support archive (ifconfig, public IP, ping)
- **bmc-support** — the platform-agnostic support-archive engine: `SupportConfig`, streamed `SupportArchive`, the
  `SupportFilter`/`SupportExtension` traits and archive formats
- **bmc-support-openwrt** — the OpenWrt board's shared credential filters and the Nix profile and `logread` extensions,
  assembled by each binary into its own `SupportConfig`
- **bmc-shared/time** — timezone handling
- **bmc-shared/utils** — number formatting, helpers

### Dev/test

- **bmc-virt** — QEMU x86/64 emulation environment (flake.nix)
- **bmc-virt-relay** — guest daemon: captures the compositor's output as a Wayland client (via
  `ext-image-copy-capture-v1`) plus SPI LED data, and serves both over TCP IPC
- **bmc-virt-console** — host-side native viewer (egui): display, touch injection, LED glow, controls
- **bmc-virt-ipc** — typed TCP protocol between relay and console
- **bmc-virt-leds** — APA102 SPI stream decoder for LED visualization

## Threading Model

The application runs two threads:

1. **Tokio async runtime** (main thread) — runs all business logic as async tasks: screen state transitions, widget data
   fetching, web/gRPC servers, WiFi management, LED control.

2. **Compositor thread** (spawned std::thread in `bmc-openwrt/src/compositor/egl_compositor.rs`) — owns the Smithay
   Wayland server, the EGL/GLES2 renderer, and the DRM output. Driven by a `calloop` event loop that multiplexes Wayland
   client traffic, libinput touch events, DRM vblank events, and frame-callback timers.

Communication between threads uses two channels:

- **Tokio → compositor**: `CompositorCommand` values (set active scene, configure scene cycling, register/unregister
  widgets) sent via a `calloop_channel::Channel` so they wake the compositor's `calloop` loop directly.
- **Compositor → tokio**: `WidgetAction` and `CompositorEvent` values (gestures, widget exits, surface presentations)
  sent via `tokio::mpsc::UnboundedChannel`.

### Virtual GPU caveat

Virtual GPUs (virtio-gpu) require `DRM_IOCTL_MODE_DIRTYFB` after each render to notify the host that the dumb buffer
contents changed. Real hardware with write-combine mapped memory does not need this — mmap writes are immediately
visible. Without the dirty ioctl, the display freezes after the initial modeset. See BDK-383 DEVLOG for the full
investigation.

## Display Pipeline

Physical display is 480x1280 (portrait). Widgets are separate processes acting as Wayland clients; the compositor
composites their surfaces into the final frame and presents it on the panel.

1. Widget processes connect to the compositor's Wayland socket and submit dmabuf or shm surfaces, configured via the
   custom `deck_widget` protocol (see `bmc-widget-protocol/protocol/deck-widget.xml`).
2. The compositor imports each surface as a GLES texture and composites the active scene with Smithay's `GlesRenderer`
   into a GBM-backed buffer (`XRGB8888`, double-buffered in `compositor/render/buffer_pool.rs`).
3. The composited buffer is attached to the DRM CRTC; presentation is driven by DRM atomic page-flip on vblank
   (synthetic 60 Hz tick when running headless).

## State Machine

On boot, `device_state()` checks shell flags via `/lib/functions/bos-defaults.sh`:

```
FactoryDefault → display_setup_start() → SetupStart screen → wait for WiFi setup
SetupPending   → show WiFi connect screen → wait for device setup completion
Operational    → show connect info (10s) → enable scene cycler (main widget view)
WifiReconfig   → display_setup_start() → AP mode active
```

Each transition sends `CompositorCommand`s (`SetActiveScene`, `SetSceneCycling`, widget enable/disable) to the
compositor thread, which updates which widget surfaces are visible and which scenes participate in swipe-driven cycling
(see `bmc-openwrt/src/compositor/widget_tracker.rs`).

## Build System

- **Rust workspace** built via Nix (naersk). Cross-compile targets: `armv7-unknown-linux-musleabihf` (production),
  `x86_64-unknown-linux-musl` (QEMU emulation).
- **Frontend** — React SPA built via Nix, served at `:80/www/bmc/`.
- **OpenWrt image** — built from source via Nix flake (`bmc-virt/flake.nix`). Custom kernel with
  `CONFIG_PROC_PAGE_MONITOR=y` for rr debugger support.
- **Compositor** — Smithay-based Wayland server in `bmc-openwrt/src/compositor/`, rendering through EGL/GLES2 onto a
  DRM/GBM output. Widgets are separate Wayland-client crates under `widgets/`, packaged into `lib/bmc-widgets`.

## Emulation (bmc-virt)

The `bmc-virt/` directory provides a QEMU x86/64 environment that runs the same binary as ARM production. Key
differences from real hardware:

| Feature       | ARM (production)                   | x86 QEMU                                         |
| ------------- | ---------------------------------- | ------------------------------------------------ |
| DRM           | real display controller            | VKMS (software, MAP_DUMB capable)                |
| Display       | physical 480x1280                  | native console app (egui) via TCP IPC            |
| Touch         | real touchscreen (evdev event0)    | virtio-tablet-pci (swapped to event0 at boot)    |
| WiFi          | Realtek USB adapter                | mac80211_hwsim (2 radios, AP + STA)              |
| LED           | APA102 via /dev/spidev0.0          | SPI kernel module → /proc capture → console glow |
| Backlight     | real sysfs                         | fake tmpfs bind mount (inotify-watched)          |
| Reset button  | GPIO USR_BTN (safety pin)          | console hold button → netlink uevent injection   |
| Factory reset | `bos factory_reset` (wipe overlay) | `bos` stub (restore flags + reboot)              |
| Debugging     | SSH + logs                         | SSH + logs + rr time-travel debugger             |

The VM is fully declarative — init.d scripts handle device setup, service management, and WiFi config on every boot.
One-command workflow: `./scripts/run.sh --customer` builds everything, boots the VM, deploys, and connects. See
`bmc-virt/README.md` for full details.

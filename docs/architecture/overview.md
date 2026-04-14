# Braiins Deck — Architecture Overview

Braiins Deck is a standalone Bitcoin dashboard display. It shows real-time market data, mining stats, and customizable
widgets on a 1280x480 touchscreen. The hardware is an STM32MP157C (dual Cortex-A7) running OpenWrt Linux.

## Repositories

| Repo         | What                                        | Where                        |
| ------------ | ------------------------------------------- | ---------------------------- |
| **bmc-main** | Deck application firmware (this repo)       | gitlab / forge/deck/bmc-main |
| **bos-main** | BraiinsOS mining firmware (bosminer, boser) | gitlab / bos/bos-main        |
| **openwrt**  | Braiins OpenWrt fork with board support     | gitlab / bos/openwrt         |

bmc-main and bos-main share some libraries (`ii-net`, `ii-net-drv`, `time`, etc.) via the `bmc-shared/` workspace member
which mirrors `open/utils-rs/` from bos-main.

## Key Crates

### Platform layer

- **bmc-openwrt** — the main binary. Platform glue: DRM rendering, WiFi detection, backlight, LED driver, touch input,
  CLI. Cross-compiled as a static musl binary for ARM (production) and x86_64 (QEMU emulation).

### Core logic

- **bmc** — state machine, display tasks (screen transitions), widget tasks (data fetching), startup orchestration, web
  server, gRPC API.

### Display

- **bmc-display** — Slint UI components, `DisplayController` (the bridge between async tasks and the Slint event loop),
  `Proxy` (flume channel implementing Slint's `EventLoopProxy`), scene management, widget rendering.

### Hardware drivers

- **bmc-led** — APA102 addressable LED strip over SPI
- **bmc-button** — hardware button via uevent
- **bmc-gpio** — GPIO via sysfs/gpiod
- **bmc-kobject** — kernel uevent listener

### Shared

- **bmc-shared/ii-net-drv** — WiFi management via OpenWrt UCI/ubus
- **bmc-shared/time** — timezone handling
- **bmc-shared/utils** — number formatting, helpers

### Dev/test

- **bmc-mock** / **bmc-mock-display** — desktop mock for development (winit backend)
- **bmc-virt** — QEMU x86/64 emulation environment (flake.nix)
- **bmc-virt-relay** — guest daemon: captures DRM framebuffer + SPI LED data, serves via TCP IPC
- **bmc-virt-console** — host-side native viewer (egui): display, touch injection, LED glow, controls
- **bmc-virt-ipc** — typed TCP protocol between relay and console
- **bmc-virt-leds** — APA102 SPI stream decoder for LED visualization

## Threading Model

The application runs two threads:

1. **Tokio async runtime** (main thread) — runs all business logic as async tasks: screen state transitions, widget data
   fetching, web/gRPC servers, WiFi management, LED control.

2. **Slint event loop** (spawned std::thread) — owns the `MinimalSoftwareWindow` and DRM framebuffer. Processes events
   from a flume channel, renders via software renderer, writes pixels to the DRM dumb buffer.

Communication between threads is through `DisplayController.in_event_loop()` which calls Slint's `upgrade_in_event_loop`
→ sends a closure through the flume `Proxy` → executed on the Slint thread.

### Virtual GPU caveat

Virtual GPUs (virtio-gpu) require `DRM_IOCTL_MODE_DIRTYFB` after each render to notify the host that the dumb buffer
contents changed. Real hardware with write-combine mapped memory does not need this — mmap writes are immediately
visible. Without the dirty ioctl, the display freezes after the initial modeset. See BDK-383 DEVLOG for the full
investigation.

## Display Pipeline

Physical display is 480x1280 (portrait). The application renders at 1280x480 (landscape) with
`RenderingRotation::Rotate270` applied by the Slint software renderer.

1. Slint renders into an in-memory `Rgb565Pixel` buffer (480 * 1280 pixels)
2. The buffer is converted to the DRM framebuffer format:
   - ARM hardware: RGB565 direct copy
   - QEMU virtio-gpu: RGB565 → XRGB8888 pixel conversion (virtio-gpu doesn't support RGB565)
3. Pixels are written row-by-row to the mmap'd DRM dumb buffer
4. DRM atomic modeset presents the buffer

## State Machine

On boot, `device_state()` checks shell flags via `/lib/functions/bos-defaults.sh`:

```
FactoryDefault → display_setup_start() → SetupStart screen → wait for WiFi setup
SetupPending   → show WiFi connect screen → wait for device setup completion
Operational    → show connect info (10s) → enable scene cycler (main widget view)
WifiReconfig   → display_setup_start() → AP mode active
```

Each transition sets Slint global adapter properties (`ScreenAdapter.init`, `ScreenAdapter.scene_cycler`, etc.) via
`invoke_from_event_loop`.

## Build System

- **Rust workspace** built via Nix (naersk). Cross-compile targets: `armv7-unknown-linux-musleabihf` (production),
  `x86_64-unknown-linux-musl` (QEMU emulation).
- **Frontend** — React SPA built via Nix, served at `:80/www/bmc/`.
- **OpenWrt image** — built from source via Nix flake (`bmc-virt/flake.nix`). Custom kernel with
  `CONFIG_PROC_PAGE_MONITOR=y` for rr debugger support.
- **Slint** — pinned at 1.13.1 with `renderer-software` feature. UI defined in `.slint` files, compiled at build time.

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

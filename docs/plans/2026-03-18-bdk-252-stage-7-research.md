# BDK-252 Stage 7: Research — Slint Screens, bmc-openwrt Patterns, WiFi Setup

> Investigation of the master branch codebase to inform `bmc-nix-init` implementation.

---

## 1. Existing Init Setup Screens

Located in `bmc-display/ui/screens/init_setup/`, the codebase already has **7 init setup screens** used for the initial WiFi configuration flow:

| Screen | File | Purpose |
|--------|------|---------|
| `InitStartConnect` | `init_start_connect.slint` | "Connect to Braiins Deck WiFi" — AP SSID + QR code |
| `InitWifiConnectProgress` | `init_wifi_connect_progress.slint` | "Connecting..." with WiFi SSID |
| `InitWifiConnectSuccess` | `init_wifi_connect_success.slint` | "You're connected" |
| `InitWifiConnectFailed` | `init_wifi_connect_failed.slint` | "Problem with connection" |
| `InitGeneralError` | `init_general_error.slint` | "Problem Occurred. Restarting." |
| `InitDeviceSetupQr` | `init_device_setup_qr.slint` | QR code for browser setup |
| `InitSetupSuccess` | `init_setup_success.slint` | "Braiins Deck is ready!" |

Mapped via `InitScreen` enum in `bmc-display/src/data.rs:673`:

```rust
pub enum InitScreen {
    SetupStart,           // → StartConnect
    SetupWifiConnecting,  // → WifiConnectProgress
    SetupWifiConnected,   // → WifiConnectSuccess
    SetupWifiError,       // → WifiConnectFailed
    SetupGeneralError,    // → GeneralError
    SetupConnectInfo,     // → DeviceSetupQr
    SetupCompleted,       // → SetupSuccess
}
```

Switched by `set_init_screen()` in `display_controller/state.rs:937`.

---

## 2. Template Architecture

Screens use **reusable templates** in `ui/screens/templates/`:

- **`InitSetupTemplate`** (`init_setup.slint`) — Black background, header title,
  icon, main title, content text, subtitle. Most init screens inherit from this.
- **`IconAndTextTemplate`** (`icon_and_text.slint`) — Simpler variant with just
  icon + text.
- **`WifiConnectProgress`** (`wifi_connect_progress.slint`) — Extends
  `InitSetupTemplate` with WiFi adapter bindings.

Style system:

- `ui/style/palette.slint` — Colors (white, black, gray-40/50/80, violet-50, etc.)
- `ui/style/theme.slint` — Font weights: regular (400), semi-bold (600), bold (700).
  Font families: BraiinsSans, BraiinsDeckSans
- `ui/style/images.slint` — SVG assets including init_setup icons (wifi-connect,
  wifi, wifi-error, refresh-big, success-big, desktop, bmc-icon, etc.)

---

## 3. Upgrade Download Screen — The Progress Bar Pattern

`bmc-display/ui/screens/upgrade/upgrade_download.slint` has exactly what we need
for tarball downloads:

```slint
import { ProgressIndicator } from "std-widgets.slint";

export component UpgradeDownload inherits Rectangle {
    background: Palette.black;

    ProgressIndicator {
        height: 7px;
        progress: UpgradeDownloadAdapter.progress * 100%;
    }

    Text { text: UpgradeDownloadAdapter.progress_text; }
    Text { text: UpgradeDownloadAdapter.downloaded_mb_text; }
}
```

Binds to `UpgradeDownloadAdapter.progress` (float 0..1), `progress_text`, and
`downloaded_mb_text`. This is the pattern to follow for the `Downloading` state.

---

## 4. `LinuxDrmPlatform` — The Key Module to Extract

`bmc-openwrt/src/linux_drm_platform.rs` (430 lines) is the **complete standalone
DRM display platform**. Key characteristics:

- Uses **Slint's `MinimalSoftwareWindow`** with `RepaintBufferType::NewBuffer`
- Opens `/dev/dri/card1` for display, `/dev/input/event0` for touch
- **Pixel format: `Rgb565`** (16 bpp), resolution 480x1280 portrait
- **`RenderingRotation::Rotate270`** → 1280x480 logical
- Atomic DRM modesetting (UniversalPlanes + Atomic capabilities)
- Double-buffering via in-memory copy (no GPU/EGL/page flip)
- Defers modeset until first frame rendered (seamless kernel splash → Slint)
- Uses `flume` channels for `Proxy`/`ProxyEvent` cross-thread dispatch
- Touch event handling via evdev (absolute axes + BTN_TOUCH)

Dependencies: `drm`, `drm-fourcc`, `slint`, `evdev`, `flume`, `bmc_display::proxy`

This is **exactly what `bmc-nix-init-openwrt` needs** — a standalone Slint platform
that doesn't depend on the EGL compositor or Wayland.

---

## 5. `bmc-openwrt/src/main.rs` — Binary Startup Pattern

```
main()
  ├─ Parse CLI, init logging, set panic hook
  ├─ Init backlight + LED drivers
  ├─ Create DisplayMetadata (1280x480 logical)
  ├─ std::thread::spawn → run_slint_platform()
  │    ├─ LinuxDrmPlatform::new(480, 1280, Rotate270)
  │    ├─ slint::platform::set_platform(drm_platform)
  │    ├─ DisplayController::create(1280, 480)
  │    ├─ Send DisplayController to main thread via flume
  │    └─ main_window.run()  ← blocks on Slint event loop
  ├─ Receive DisplayController from flume channel
  ├─ Init WiFi manager (OpenwrtWifiManager), session manager
  ├─ manager.init_wifi_ap()
  └─ bmc::entry::main(...)
```

Key device paths:

- Backlight: `/sys/class/backlight/display-bl`
- LED SPI: `/dev/spidev0.0`
- DRM display: `/dev/dri/card1`
- Touch input: `/dev/input/event0`
- Board detection: `/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/idVendor`

---

## 6. WiFi Manager Pattern

### Architecture Layers

- **ii-net** (`bmc-shared/ii-net/src/wifi.rs`) — Pure data types: `WifiMode`,
  `EncryptionType`, `WifiStatus`, `WifiScanItem`
- **ii-net-drv** (`bmc-shared/ii-net-drv/src/wifi/`) — OpenWRT WiFi driver:
  `OpenwrtWifiManager`, `UciHelper`, `WifiScanner`, `WifiSta`
- **bmc/manager** (`bmc/src/manager.rs`) — `BmcManager` trait, `WifiNetworkConfig`,
  `BmcState` enum
- **bmc-openwrt/manager** (`bmc-openwrt/src/manager.rs`) — Real implementation

### AP Mode Setup (from `bmc-openwrt/src/manager.rs`)

```rust
async fn configure_wifi_ap() -> Result<(), InitialSetupError> {
    wifi_manager.reset_config().await?;
    // configure_radio_for_ap: channel 1, beacon 500, NOHT mode
    wifi_manager.configure_radio_for_ap().await?;
    // configure_wifi_iface: AP mode, ssid_with_mac, encryption=None
    wifi_manager.configure_wifi_iface(WifiMode::AP, ssid, None, "").await?;
    wifi_manager.enable_radio(true).await?;
}
```

SSID generation: base name ("Braiins Deck") + last 3 chars of MAC address.

### Station Mode Connection

```rust
async fn wifi_save_and_connect(ssid, password, encryption) {
    // Configure STA mode via UciHelper
    // Enable radio + WifiCommand::reload()
    // Wait for IP (30 attempts, 1 sec intervals)
}
```

### Board Detection

```rust
let vendor = read("/sys/devices/.../usb3/3-1/idVendor");
let wifi_path = if vendor == "0bda" {
    WIFI_PATH_HUBLESS  // /sys/devices/.../3-1:1.0/
} else {
    WIFI_PATH_HUBBED   // /sys/devices/.../3-1.1/3-1.1:1.0/
};
let wifi_manager = OpenwrtWifiManager::new(wifi_path)?;
```

### Key Constants

```rust
const WIFI_AP_CHANNEL: u32 = 1;
const WIFI_AP_BEACON_INTERVAL: u32 = 500;
const ATTEMPTS_TO_GET_IP: u8 = 30;
const IP_CHECK_INTERVAL: Duration = Duration::from_secs(1);
```

---

## 7. Initial Setup Flow (`bmc/src/initial_setup.rs`)

The `InitialSetup<T, F>` service orchestrates setup with a state machine:

```
FactoryDefault → init_wifi_ap() → captive portal
  User configures WiFi via frontend gRPC
    → InitialSetup::connect_to_wifi()
    → BmcManager::wifi_initial_setup()
    → disable captive portal
SetupPending → user completes device setup
Operational
```

State tracked via `watch::Receiver<Option<InitSetupState>>` and displayed by
`display_tasks.rs` calling `display_controller.set_init_screen()`.

---

## 8. Mapping Stage 7 `InitState` to Screens

| InitState | UI Approach | Reuse from |
|-----------|------------|------------|
| `Checking` / `AlreadyInitialized` | **No screen** (don't init display) | — |
| `NoWifi { ap_ssid }` | AP SSID + QR code | `InitStartConnect` pattern |
| `Connecting` | "Connecting..." + SSID | `WifiConnectProgress` pattern |
| `FetchingIndex` | "Checking for updates..." | `InitSetupTemplate` + spinner |
| `Downloading { bytes, total }` | Progress bar + MB counter | `UpgradeDownload` pattern |
| `Extracting` | "Installing packages..." | `UpgradeProgress` pattern |
| `Activating` | "Finalizing..." | `InitSetupTemplate` simple |
| `Done` | "Ready!" | `InitSetupSuccess` pattern |
| `Error { message }` | Error + message | `InitGeneralError` pattern |

---

## 9. Key Architecture Decision: Separate Slint UI

The init binary **cannot depend on the full `bmc-display` crate** — it compiles
the entire `main.slint` with all 50+ screens (~50MB generated code). Instead:

1. **Create a minimal `bmc-nix-init/ui/init.slint`** with only the ~8 screens
   needed for the init flow.
2. **Copy the template patterns** (palette, theme, `InitSetupTemplate`) into the
   init crate's UI directory. Keep them minimal — only include colors, fonts, and
   image assets actually used.
3. **Extract `LinuxDrmPlatform`** into `bmc-nix-init-openwrt/` (or create a
   shared crate). The `Proxy`/`ProxyEvent` module from `bmc-display` is small
   enough to duplicate.
4. **Threading model**: Follow the existing `bmc-openwrt/main.rs` pattern — Slint
   event loop on a dedicated `std::thread`, tokio on main thread, `flume` channel
   to send the window handle, `slint::invoke_from_event_loop()` for state updates.

### What to reuse

- `LinuxDrmPlatform` — self-contained DRM platform, copy into openwrt crate
- Init screen template patterns — `InitSetupTemplate` layout and style
- `UpgradeDownload` progress bar pattern — `ProgressIndicator` from std-widgets
- `OpenwrtWifiManager` from `bmc-shared/ii-net-drv` — handles all WiFi ops
- AP mode setup pattern from `bmc-openwrt/src/manager.rs`

### What NOT to reuse

- The full `bmc-display` crate — too heavy, compiles all screens
- The EGL compositor (`bmc-openwrt/src/compositor/`) — not needed,
  `LinuxDrmPlatform` is sufficient for software rendering
- The `DisplayController` abstraction — too coupled to the full UI
- The `BmcManager` mega-trait — use the narrower `WifiManager` trait instead

---

## 10. Key Reference Files

| File | Purpose |
|------|---------|
| `bmc-openwrt/src/linux_drm_platform.rs` | DRM + Slint software renderer platform |
| `bmc-openwrt/src/main.rs` | Binary startup pattern, Slint thread spawning |
| `bmc-display/build.rs` | Slint build config with `EmbedForSoftwareRenderer` |
| `bmc-display/ui/screens/templates/init_setup.slint` | Init screen template |
| `bmc-display/ui/screens/upgrade/upgrade_download.slint` | Progress bar pattern |
| `bmc-display/ui/style/palette.slint` | Color definitions |
| `bmc-display/ui/style/theme.slint` | Font weights |
| `bmc-display/ui/style/images.slint` | SVG asset references |
| `bmc-display/src/data.rs:673` | `InitScreen` enum |
| `bmc-display/src/display_controller/state.rs:937` | `set_init_screen()` |
| `bmc-openwrt/src/manager.rs` | WiFi AP setup, `configure_wifi_ap()` |
| `bmc/src/initial_setup.rs` | Initial setup orchestration, state machine |
| `bmc-shared/ii-net-drv/src/wifi/` | `OpenwrtWifiManager`, UCI helpers |
| `bmc/src/display_tasks.rs` | Display task spawning, init screen management |

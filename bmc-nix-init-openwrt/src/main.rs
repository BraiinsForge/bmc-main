// Copyright (C) 2026  Braiins Systems s.r.o.

mod linux_drm_platform;
mod platform;

use bmc_nix_init::app::run_app;
use bmc_nix_init::config::InitConfig;
use bmc_nix_init::init::InitPlatform;
use bmc_platform::backlight::DisplayBacklightDriver as _;
use bmc_platform::generic_backlight_driver::GenericBacklightDriver;
use clap::Parser;
use linux_drm_platform::LinuxDrmPlatform;
use slint::platform::software_renderer::RenderingRotation;

// TODO: These WiFi path constants and detect_wifi_path() are
// duplicated from bmc-openwrt/src/main.rs. Extract into bmc-platform.

/// Realtek WiFi adapter vendor ID
const WIFI_VENDOR_ID: &str = "0bda";

/// Shared USB device path (exists on both hubbed and hubless boards)
const SHARED_USB_DEVICE: &str = "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/";

/// WiFi interface paths for each board type
const WIFI_PATH_HUBLESS: &str = "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/3-1:1.0/";
const WIFI_PATH_HUBBED: &str =
    "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/3-1.1/3-1.1:1.0/";

#[derive(Parser)]
#[command(name = "bmc-nix-init")]
struct Cli {
    /// Path to servers.json configuration
    #[arg(long, default_value = "/etc/nix-upgrade/servers.json")]
    servers_config: std::path::PathBuf,

    /// Path to BOS version file
    #[arg(long, default_value = "/etc/bos_version")]
    bos_version_path: std::path::PathBuf,

    /// Profile directory
    #[arg(long, default_value = "/nix/var/nix/gcroots/profiles/bmc")]
    profile_dir: std::path::PathBuf,

    /// WiFi device sysfs path (auto-detected if omitted)
    #[arg(long)]
    wifi_path: Option<String>,
}

fn main() {
    tracing_subscriber::fmt::init();
    let args = Cli::parse();

    let config = InitConfig {
        servers_config_path: args.servers_config,
        bos_version_path: args.bos_version_path,
        profile_dir: args.profile_dir,
        ..Default::default()
    };

    let wifi_path = args.wifi_path.unwrap_or_else(detect_wifi_path);
    tracing::info!("using WiFi device path: {wifi_path}");

    let platform = std::sync::Arc::new(platform::OpenwrtPlatform::new(wifi_path));

    // Fast path: check if init is needed before taking over the display.
    if config.inhibit_init_path.exists() {
        tracing::info!("init inhibited by {}", config.inhibit_init_path.display());
        return;
    }
    if platform.is_store_ever_initialized(&config) {
        tracing::info!("store already initialized, exiting");
        return;
    }

    set_default_brightness();

    // Physical display is 600x1280 (portrait). Pass height x width
    // with Rotate270 to get a 1280x480 logical display.
    let drm = LinuxDrmPlatform::new(480, 1280, RenderingRotation::Rotate270);
    slint::platform::set_platform(Box::new(drm)).expect("BUG: failed to set Slint platform");

    run_app(config, platform);
}

/// Set the display backlight to 60% of max before the first frame is rendered.
/// On any error, logs a warning and returns — the display will still work.
fn set_default_brightness() {
    let mut backlight = GenericBacklightDriver::new("/sys/class/backlight/display-bl");
    if let Err(e) = backlight.init() {
        tracing::warn!("cannot init backlight driver: {e}");
        return;
    }
    if let Err(e) = backlight.set_brightness_pct(60) {
        tracing::warn!("cannot set brightness: {e}");
        return;
    }
    tracing::info!("display brightness set to 60%");
}

/// Detect WiFi device sysfs path using the same hubbed/hubless board
/// detection as `bmc-openwrt`. Reads the USB vendor ID at the shared
/// device path to distinguish the two board variants.
/// TODO: this should be based on OTP, not sysfs paths.
fn detect_wifi_path() -> String {
    let vendor = std::fs::read_to_string(format!("{SHARED_USB_DEVICE}idVendor"))
        .map(|v| v.trim().to_owned())
        .unwrap_or_default();

    if vendor == WIFI_VENDOR_ID {
        tracing::info!("hubless board detected, WiFi at {WIFI_PATH_HUBLESS}");
        WIFI_PATH_HUBLESS.to_owned()
    } else {
        tracing::info!("hubbed board detected, WiFi at {WIFI_PATH_HUBBED}");
        WIFI_PATH_HUBBED.to_owned()
    }
}

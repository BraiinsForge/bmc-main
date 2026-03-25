// Copyright (C) 2026  Braiins Systems s.r.o.

use std::panic;
use std::path::PathBuf;
use std::{str::FromStr, sync::Arc};

use anyhow::Result;
use bmc::backlight::DisplayBacklightDriver;
use bmc::compositor::Compositor;
use bmc::{BmcManager, Configuration};
use bmc_led::apa102_spi::platform_led_driver::PlatformLedDriver;
use bmc_led::led_driver::LedDriverFactory;
use bmc_openwrt::cli::Parser;
use bmc_openwrt::compositor::EglCompositor;
use bmc_openwrt::{
    button_driver::UEventButtons, generic_backlight_driver::GenericBacklightDriver,
    manager::Manager, session::OpenwrtSessionManager,
};
use bmc_openwrt::{cli::Args, log::build_panic_hook_with_tracing};
use bmc_shared_ii_net_drv::wifi::OpenwrtWifiManager;
use bmc_shared_time::time::Timezone;
use tokio::sync::Mutex;
use tracing::{error, info};

/// Realtek WiFi adapter vendor ID
const WIFI_VENDOR_ID: &str = "0bda";

/// Shared USB device path (exists on both hubbed and hubless boards)
const SHARED_USB_DEVICE: &str = "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/";

/// WiFi interface paths for each board type
const WIFI_PATH_HUBLESS: &str = "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/3-1:1.0/";
const WIFI_PATH_HUBBED: &str =
    "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/3-1.1/3-1.1:1.0/";

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    bmc_openwrt::log::init(args.log_to_file);

    panic::set_hook(build_panic_hook_with_tracing());

    let mut backlight_driver = GenericBacklightDriver::new("/sys/class/backlight/display-bl");
    backlight_driver.init()?;
    let backlight_driver = Arc::new(Mutex::new(backlight_driver));

    let led_driver = PlatformLedDriver::new("/dev/spidev0.0");

    let config = Configuration {
        widgets_paths: args
            .widgets_paths
            .unwrap_or_else(|| vec![PathBuf::from("/run/current-profile/lib/bmc-widgets")]),
        ..Configuration::default()
    };

    let bmc_index = bmc::firmware::BmcIndex::default();

    let current_timezone = iana_time_zone::get_timezone()
        .ok()
        .and_then(|timezone| Timezone::from_str(&timezone).ok())
        .unwrap_or_default();

    // BMC_WIFI_SYSPATH overrides auto-detection (used for x86 QEMU emulation with mac80211_hwsim).
    // Otherwise detect board type by checking vendor ID at the shared USB path:
    // - Hubless: 3-1 is the WiFi (vendor 0bda)
    // - Hubbed: 3-1 is the USB hub, WiFi is at 3-1.1
    let wifi_path: String = if let Ok(path) = std::env::var("BMC_WIFI_SYSPATH") {
        info!("Using WiFi device path from BMC_WIFI_SYSPATH: {}", path);
        path
    } else {
        let vendor = std::fs::read_to_string(format!("{SHARED_USB_DEVICE}idVendor"))
            .map(|v| v.trim().to_owned())
            .unwrap_or_default();

        info!("Detecting WiFi: {}idVendor = {}", SHARED_USB_DEVICE, vendor);

        if vendor == WIFI_VENDOR_ID {
            info!("Hubless board detected, WiFi at {}", WIFI_PATH_HUBLESS);
            WIFI_PATH_HUBLESS.to_owned()
        } else {
            info!("Hubbed board detected, WiFi at {}", WIFI_PATH_HUBBED);
            WIFI_PATH_HUBBED.to_owned()
        }
    };

    info!("Using WiFi device path: {}", wifi_path);
    let wifi_manager = Arc::new(
        OpenwrtWifiManager::new(&wifi_path)
            .inspect_err(|err| error!(?err, "Failed to initialize WiFi Manager"))?,
    );

    let manager = Manager::new(
        OpenwrtSessionManager,
        current_timezone,
        wifi_manager,
        "Braiins Deck".to_owned(),
    )
    .await;

    // Has check on factory default already
    if let Err(err) = manager.init_wifi_ap().await {
        error!(?err, "Failed to setup init WiFi AP");
    }

    // Initialize and start the EGL compositor
    let compositor = Arc::new(EglCompositor::new(args.headless_compositor));
    let wayland_display = compositor
        .start()
        .expect("BUG: failed to start EGL compositor");
    info!("Compositor started on {}", wayland_display);

    bmc::entry::main(
        manager,
        config,
        backlight_driver,
        led_driver.0,
        bmc_index,
        Arc::new(Box::new(UEventButtons)),
        compositor,
    )
    .await?;

    Ok(())
}

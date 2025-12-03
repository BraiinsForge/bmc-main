// Copyright (C) 2025  Braiins Systems s.r.o.

use std::panic;
use std::{str::FromStr, sync::Arc};

use anyhow::{Context, Result, anyhow};
use bmc::{BmcManager, Configuration};
use bmc_display::{
    display_controller::DisplayController,
    display_driver::{DisplayBacklightDriver, DisplayDriver},
    metadata::{DisplayMetadata, ResolutionMetadata, UsizeMetadata},
};
use bmc_led::led_driver::LedDriverFactory;
use bmc_openwrt::cli::Parser;
use bmc_openwrt::{
    button_driver::UEventButtons, generic_backlight_driver::GenericBacklightDriver,
    linux_drm_platform::LinuxDrmPlatform, manager::Manager, session::OpenwrtSessionManager,
};
use bmc_openwrt::{
    cli::Args, led_driver::platform_led_driver::PlatformLedDriver,
    log::build_panic_hook_with_tracing,
};
use bmc_shared_ii_net_drv::wifi::OpenwrtWifiManager;
use bmc_shared_time::time::Timezone;
use bmc_upgrade::firmware::FirmwareResolver;
use slint::platform::software_renderer::RenderingRotation;
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

    let led_driver = PlatformLedDriver::new("/dev/spidev0.0");

    //TODO: this will be read from config file or emmc
    let brightness = UsizeMetadata::new(18, 0, 20);
    let resolution = ResolutionMetadata::new(1280, 480);
    let display_metadata = DisplayMetadata::new(brightness, resolution);

    let display_controller = get_display_controller(display_metadata)?;

    let display_driver = DisplayDriver::init(backlight_driver, display_controller)?;

    let config = Configuration::default();

    let bmc_index = bmc::firmware::BmcIndex::default();
    let firmware_resolver = FirmwareResolver::new(bmc_index);

    let current_timezone = iana_time_zone::get_timezone()
        .ok()
        .and_then(|timezone| Timezone::from_str(&timezone).ok())
        .unwrap_or_default();

    // Detect board type by checking vendor ID at shared USB path
    // - Hubless: 3-1 is the WiFi (vendor 0bda)
    // - Hubbed: 3-1 is the USB hub, WiFi is at 3-1.1
    let vendor = std::fs::read_to_string(format!("{SHARED_USB_DEVICE}idVendor"))
        .map(|v| v.trim().to_owned())
        .unwrap_or_default();

    info!("Detecting WiFi: {}idVendor = {}", SHARED_USB_DEVICE, vendor);

    let wifi_path = if vendor == WIFI_VENDOR_ID {
        info!("Hubless board detected, WiFi at {}", WIFI_PATH_HUBLESS);
        WIFI_PATH_HUBLESS
    } else {
        info!("Hubbed board detected, WiFi at {}", WIFI_PATH_HUBBED);
        WIFI_PATH_HUBBED
    };

    info!("Using WiFi device path: {}", wifi_path);
    let wifi_manager = Arc::new(OpenwrtWifiManager::new(wifi_path)?);

    let manager = Manager::new(
        OpenwrtSessionManager,
        current_timezone,
        wifi_manager,
        "Braiins Deck".to_owned(),
    );

    // Has check on factory default already
    if let Err(err) = manager.init_wifi_ap().await {
        error!(?err, "Failed to setup init WiFi AP");
    }

    bmc::entry::main(
        manager,
        config,
        display_driver,
        led_driver.0,
        firmware_resolver,
        Arc::new(Box::new(UEventButtons)),
    )
    .await?;

    Ok(())
}

fn get_display_controller(display_metadata: DisplayMetadata) -> Result<DisplayController> {
    let (ui_handle_sender, ui_handle_receiver) = flume::unbounded();

    // Spawn a thread to initiaize linux platform
    std::thread::spawn(move || {
        if let Err(e) = run_slint_platform(&display_metadata, &ui_handle_sender) {
            error!("{:#}", e);
        }
    });

    // Wait for the UI handle to be created
    ui_handle_receiver
        .recv()
        .map_err(|e| anyhow!("Cannot receive slint handle: {:#}", e))
}

fn run_slint_platform(
    display_metadata: &DisplayMetadata,
    ui_handle_sender: &flume::Sender<DisplayController>,
) -> Result<()> {
    info!("Setting up slint platform for linux framebuffer display");
    slint::platform::set_platform(Box::new(
        LinuxDrmPlatform::new(
            display_metadata.resolution.height as usize,
            display_metadata.resolution.width as usize,
            RenderingRotation::Rotate270,
        )
        .context("Cannot create platform")?,
    ))
    .map_err(|e| anyhow!("Cannot set platform: {e:#?}"))?;

    // Initialize the UI
    let (display_controller, main_window) = DisplayController::create(
        display_metadata.resolution.width,
        display_metadata.resolution.height,
    )
    .context("Cannot initialize ui")?;

    // Send the UI handle to the main thread to create the display controller
    ui_handle_sender
        .send(display_controller)
        .context("Cannot send ui_handle")?;

    // Run the event loop
    main_window.run().context("Cannot run event loop")?;
    Ok(())
}

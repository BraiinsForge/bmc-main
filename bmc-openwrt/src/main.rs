// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{str::FromStr, sync::Arc};

use anyhow::{Context, Result, anyhow};
use bmc::{BmcManager, Configuration, log};
use bmc_display::{
    display_controller::DisplayController,
    display_driver::{DisplayBacklightDriver, DisplayDriver},
    metadata::{DisplayMetadata, ResolutionMetadata, UsizeMetadata},
};
use bmc_led::led_driver::LedDriverFactory;
use bmc_openwrt::led_driver::platform_led_driver::PlatformLedDriver;
use bmc_openwrt::{
    button_driver::UEventButtons, generic_backlight_driver::GenericBacklightDriver,
    linux_drm_platform::LinuxDrmPlatform, manager::Manager, session::OpenwrtSessionManager,
};
use bmc_shared_ii_net_drv::wifi::OpenwrtWifiManager;
use bmc_shared_time::time::Timezone;
use bmc_upgrade::firmware::FirmwareResolver;
use slint::platform::software_renderer::RenderingRotation;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    log::init();
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

    let wifi_manager = Arc::new(OpenwrtWifiManager::new(
        "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/3-1:1.0/", // TODO: This is pre-prod board specific
    )?);

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

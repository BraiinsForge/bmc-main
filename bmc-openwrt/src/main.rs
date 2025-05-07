// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::{Context, Result, anyhow};
use async_trait as _;
use axum_extra as _;
use bmc::{Configuration, log};
use bmc_display::{
    display_driver::{DisplayBacklightDriver, DisplayDriver},
    metadata::{DisplayMetadata, ResolutionMetadata, UsizeMetadata},
    slint_handle::SlintHandle,
};
use bmc_openwrt::{
    generic_backlight_driver::GenericBacklightDriver,
    linux_framebuffer_platform::LinuxFbPlatform,
    manager::{Manager, OpenwrtSessionManager},
};
use bmc_platform as _;
use bmc_upgrade::firmware::FirmwareResolver;
use memmap2 as _;
use slint::ComponentHandle;
use thiserror as _;
use tracing as _;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    log::init();

    let mut backlight_driver = GenericBacklightDriver::new("/sys/class/backlight/display-bl");
    backlight_driver.init()?;

    //TODO: this will be read from config file or emmc
    let brightness = UsizeMetadata::new(18, 0, 20);
    let resolution = ResolutionMetadata::new(480, 320);
    let display_metadata = DisplayMetadata::new(brightness, resolution);

    let slint_handle = get_slint_handle(display_metadata)?;

    let display_driver = DisplayDriver::new(backlight_driver, slint_handle);

    let config = Configuration::default();

    let bmc_index = bmc::firmware::BmcIndex;
    let firmware_resolver = FirmwareResolver::new(bmc_index);

    let manager = Manager::new(OpenwrtSessionManager);

    bmc::entry::main(manager, config, display_driver, firmware_resolver).await
}

fn get_slint_handle(display_metadata: DisplayMetadata) -> Result<SlintHandle> {
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
    ui_handle_sender: &flume::Sender<SlintHandle>,
) -> Result<()> {
    info!("Setting up slint platform for linux framebuffer display");
    slint::platform::set_platform(Box::new(
        LinuxFbPlatform::new(
            display_metadata.resolution.width as usize,
            display_metadata.resolution.height as usize,
        )
        .context("Cannot create platform")?,
    ))
    .map_err(|e| anyhow!("Cannot set platform: {e:#?}"))?;

    // Initialize the UI
    let (slint_handle, main_window) = SlintHandle::create(
        display_metadata.resolution.width,
        display_metadata.resolution.height,
    )
    .context("Cannot initialize ui")?;

    // Send the UI handle to the main thread to create the display controller
    ui_handle_sender
        .send(slint_handle)
        .context("Cannot send ui_handle")?;

    // Run the event loop
    main_window.run().context("Cannot run event loop")?;
    Ok(())
}

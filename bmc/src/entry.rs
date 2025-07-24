// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{App, BmcManager, Configuration};
use anyhow::Result;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use bmc_led::led_driver::LedDriver;
use bmc_upgrade::firmware::{FirmwareIndex, FirmwareResolver};
use std::sync::Arc;

pub async fn main<T: DisplayBacklightDriver, U: FirmwareIndex>(
    manager: impl BmcManager,
    config: Configuration,
    display: DisplayDriver<T>,
    led_driver: LedDriver,
    firmware_resolver: FirmwareResolver<U>,
    buttons: Arc<Box<dyn bmc_button::Buttons + Send + Sync>>,
) -> Result<()> {
    let manager = Arc::new(manager);
    let session_manager = manager.session_manager();

    let app = App::init(
        config,
        manager,
        session_manager,
        display,
        led_driver,
        firmware_resolver,
        buttons,
    )
    .await?;
    app.run().await
}

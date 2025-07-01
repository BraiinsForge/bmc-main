// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{App, BmcManager, Configuration};
use anyhow::Result;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use bmc_led::{data::LedCommand, led_driver::LedDriver};
use bmc_upgrade::firmware::{FirmwareIndex, FirmwareResolver};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub async fn main<T: DisplayBacklightDriver, U: FirmwareIndex>(
    manager: impl BmcManager,
    config: Configuration,
    display: DisplayDriver<T>,
    led: LedDriver,
    led_cmd_tx: Sender<LedCommand>,
    firmware_resolver: FirmwareResolver<U>,
) -> Result<()> {
    let manager = Arc::new(manager);
    let session_manager = manager.session_manager();

    let app = App::init(
        config,
        manager,
        session_manager,
        display,
        led,
        led_cmd_tx,
        firmware_resolver,
    )
    .await?;
    app.run().await
}

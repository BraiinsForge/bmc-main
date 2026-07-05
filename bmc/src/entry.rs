// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::backlight::DisplayBacklightDriver;
use crate::compositor::Compositor;
use crate::{App, BmcManager, Configuration};
use anyhow::Result;
use bmc_led::led_driver::LedDriver;
use bmc_upgrade::firmware::FirmwareIndex;
use bmc_upgrade::packages::PackageBackend;
use std::sync::Arc;
use tokio::sync::Mutex;

#[expect(
    clippy::too_many_arguments,
    reason = "the entry point takes one argument per injected subsystem"
)]
pub async fn main<T: DisplayBacklightDriver, U: FirmwareIndex>(
    manager: impl BmcManager,
    config: Configuration,
    backlight_driver: Arc<Mutex<T>>,
    led_driver: LedDriver,
    firmware_index: U,
    package_backend: Arc<dyn PackageBackend>,
    buttons: Arc<Box<dyn bmc_button::Buttons + Send + Sync>>,
    compositor: Arc<dyn Compositor>,
) -> Result<()> {
    let manager = Arc::new(manager);
    let session_manager = manager.session_manager();

    let app = App::init(
        config,
        manager,
        session_manager,
        backlight_driver,
        led_driver,
        firmware_index,
        package_backend,
        buttons,
        compositor,
    )
    .await?;
    app.run().await
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{App, BmcManager, Configuration};
use anyhow::Result;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use bmc_upgrade::firmware::{FirmwareIndex, FirmwareResolver};
use std::sync::Arc;
use tracing::error;

pub async fn main<T: DisplayBacklightDriver, U: FirmwareIndex>(
    manager: impl BmcManager,
    config: Configuration,
    display: DisplayDriver<T>,
    firmware_resolver: FirmwareResolver<U>,
) -> Result<()> {
    let manager = Arc::new(manager);
    let session_manager = manager.session_manager();

    let app = App::init(config, manager, session_manager, display, firmware_resolver).await?;

    _ = app.run().await.map_err(|e| error!("Error: {e}"));

    Ok(())
}

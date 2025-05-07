// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{App, BmcManager, Configuration};
use anyhow::Result;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use std::sync::Arc;
use tracing::error;

pub async fn main<T: DisplayBacklightDriver>(
    manager: impl BmcManager,
    config: Configuration,
    display: DisplayDriver<T>,
) -> Result<()> {
    let manager = Arc::new(manager);

    let display = Arc::new(display);

    let app = App::init(config, manager.clone(), manager.session_manager(), display).await?;

    _ = app.run().await.map_err(|e| error!("Error: {e}"));

    Ok(())
}

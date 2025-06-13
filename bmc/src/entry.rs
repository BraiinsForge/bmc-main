// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{App, BmcManager, Configuration};
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use bmc_upgrade::firmware::{FirmwareIndex, FirmwareResolver};
use std::process::exit;
use std::sync::Arc;
use tracing::error;

pub async fn main<T: DisplayBacklightDriver, U: FirmwareIndex>(
    manager: impl BmcManager,
    config: Configuration,
    display: DisplayDriver<T>,
    firmware_resolver: FirmwareResolver<U>,
) {
    let manager = Arc::new(manager);
    let session_manager = manager.session_manager();

    let fut = async move {
        let app = App::init(config, manager, session_manager, display, firmware_resolver).await?;
        app.run().await
    };

    if let Err(err) = fut.await {
        error!("Error: {err}");
        // intentionally kill the app, because this future might be running inside a tokio task
        exit(1);
    }
}

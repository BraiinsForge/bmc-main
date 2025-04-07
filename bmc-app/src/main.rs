// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_core::App;
use std::sync::Arc;
use tracing::error;

#[cfg(not(target_arch = "arm"))]
use bmc_mock as platform;

#[cfg(target_arch = "arm")]
use bmc_openwrt as platform;

#[tokio::main]
async fn main() -> Result<()> {
    let (manager, config) = platform::init()?;
    let manager = Arc::new(manager);

    let app = App::init(config, manager).await?;

    _ = app.run().await.map_err(|e| error!("Error: {e}"));

    Ok(())
}

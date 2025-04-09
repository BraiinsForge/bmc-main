// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{App, BmcManager, Configuration};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::error;

#[async_trait]
pub trait Initializer: Send + Sync + 'static {
    async fn initialize(self) -> Result<(impl BmcManager, Configuration)>;
}

pub async fn main<T>(initializer: T) -> Result<()>
where
    T: Initializer,
{
    let (manager, config) = initializer.initialize().await?;
    let manager = Arc::new(manager);

    let app = App::init(config, manager).await?;

    _ = app.run().await.map_err(|e| error!("Error: {e}"));

    Ok(())
}

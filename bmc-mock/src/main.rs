// Copyright (C) 2025  Braiins Systems s.r.o.
mod cli;
mod mockfs;

use anyhow::Result;
use async_trait as _;
use axum_extra as _;
use bmc::log;
use bmc_display as _;
use bmc_mock::{MockManager, MockSessionManager};
use bmc_mock_display::VirtualDisplay;
use clap as _;
use clap::Parser;
use dirs as _;
use rand as _;
use slint::ComponentHandle;
use thiserror as _;
use time as _;
use tokio as _;
use tracing as _;

#[tokio::main]
async fn main() -> Result<()> {
    log::init();

    let config = cli::Config::parse();
    let system_password = config.system_password.clone();

    let mockfs = mockfs::MockFs::new(&config.mockfs_path);
    mockfs.init()?;

    let config = config.into();

    let (main_window, display_driver) = VirtualDisplay::create()?;

    let manager = MockManager {
        session_manager: MockSessionManager::new(system_password),
    };

    tokio::task::spawn(bmc::entry::main(manager, config, display_driver));

    Ok(main_window.run()?)
}

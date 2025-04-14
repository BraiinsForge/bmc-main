// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc::log;
use bmc_display as _;
use bmc_mock::{MockManager, cli, mockfs};
use bmc_mock_display::VirtualDisplay;
use clap::Parser;
use dirs as _;
use slint::ComponentHandle;

#[tokio::main]
async fn main() -> Result<()> {
    log::init();

    let config = cli::Config::parse();

    let mockfs = mockfs::MockFs::new(&config.mockfs_path);
    mockfs.init()?;

    let config = config.into();

    let (main_window, display_driver) = VirtualDisplay::create()?;

    tokio::task::spawn(bmc::entry::main(MockManager, config, display_driver));

    Ok(main_window.run()?)
}

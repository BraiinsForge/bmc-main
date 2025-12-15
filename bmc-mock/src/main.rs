// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc::log;
use bmc_led::led_driver::LedDriverFactory;
use bmc_mock::MockSessionManager;
use bmc_mock::backlight_driver::MockBacklightDriver;
use bmc_mock::button_driver::build_buttons;
use bmc_mock::led_driver::PlatformLedDriver;
use bmc_mock::{cli, manager::Manager, mock_index::MockIndex, mockfs};
use bmc_upgrade::firmware::FirmwareResolver;
use clap::Parser;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<()> {
    log::init();
    let config = cli::Config::parse();
    let system_password = config.system_password.clone();

    let mockfs = mockfs::MockFs::new(&config.mockfs_template, &config.mockfs_path);
    mockfs.init(
        config.mockfs_reset,
        config.factory_default,
        config.setup_pending,
    )?;

    let password = Arc::new(Mutex::new(system_password));

    let manager = Manager::new(
        mockfs,
        MockSessionManager::new(password.clone()),
        password,
        config.hostname.clone(),
        config.mac_address.clone(),
        config.ip_address,
        config.address.port(),
    );

    let config = config.into();

    let backlight_driver = MockBacklightDriver::new(true, 18, 20);
    let backlight_driver = Arc::new(tokio::sync::Mutex::new(backlight_driver));

    let led_driver = PlatformLedDriver::new("");

    let firmware_resolver = FirmwareResolver::new(MockIndex);

    bmc::entry::main(
        manager,
        config,
        backlight_driver,
        led_driver.0,
        firmware_resolver,
        build_buttons(),
    )
    .await
}

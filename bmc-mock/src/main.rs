// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc::BmcManager;
use bmc::log;
use bmc_led::led_driver::LedDriverFactory;
use bmc_mock::MockSessionManager;
use bmc_mock::backlight_driver::MockBacklightDriver;
use bmc_mock::button_driver::build_buttons;
use bmc_mock::led_driver::PlatformLedDriver;
use bmc_mock::{
    cli, manager::Manager, mock_compositor::MockCompositor, mock_index::MockIndex, mockfs,
};
use bmc_platform::{BosPlatform, HardwareProfileSelection};
use bmc_upgrade::packages::{NixUpgradeConfig, PackageUpgrader};
use clap::Parser;
use std::sync::{Arc, Mutex};
use tracing::error;

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

    let platform = match config.hardware_profile.parse::<HardwareProfileSelection>() {
        Ok(HardwareProfileSelection::Platform(platform)) => platform,
        Ok(HardwareProfileSelection::Auto) => BosPlatform::Bmc1,
        Err(err) => {
            error!(%err, "invalid --hardware-profile");
            return Err(err.into());
        }
    };

    let manager = Manager::new(
        mockfs,
        MockSessionManager::new(password.clone()),
        password,
        config.hostname.clone(),
        config.mac_address.clone(),
        config.ip_address,
        config.address.port(),
        platform,
    );

    let config: bmc::Configuration = config.into();

    let package_backend = Arc::new(PackageUpgrader::new(NixUpgradeConfig {
        servers_config_path: config.nix_servers_config_path.clone(),
        profile_dir: config.nix_profile_dir.clone(),
        hooks_dir: config.nix_hooks_dir.clone(),
        hooks_override_path: config.nix_hooks_override_path.clone(),
    }));

    let backlight_driver = MockBacklightDriver::new(true, 18, 20);
    let backlight_driver = Arc::new(tokio::sync::Mutex::new(backlight_driver));

    let led_driver = PlatformLedDriver::new("");

    let compositor = Arc::new(MockCompositor::new(manager.platform().product()));

    bmc::entry::main(
        manager,
        config,
        backlight_driver,
        led_driver.0,
        MockIndex,
        package_backend,
        build_buttons(),
        compositor,
    )
    .await
}

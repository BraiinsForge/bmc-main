// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use anyhow::Result;
use bmc::BmcManager;
use bmc::log;
use bmc_led::led_driver::LedDriverFactory;
use bmc_mock::MockSessionManager;
use bmc_mock::backlight_driver::MockBacklightDriver;
use bmc_mock::button_driver::build_buttons;
use bmc_mock::led_driver::PlatformLedDriver;
use bmc_mock::mock_package_backend::MockPackageBackend;
use bmc_mock::{
    cli, manager::Manager, mock_compositor::MockCompositor, mock_index::MockIndex, mockfs,
    scenario, widget_staging,
};
use bmc_platform::{BosPlatform, HardwareProfileSelection};
use clap::Parser;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use tracing::{error, warn};

const DEFAULT_SERVICE_NAME: &str = "bmc-compositor";

fn main() -> Result<()> {
    if std::env::var_os(bmc::manager::SERVICE_NAME_ENV).is_none() {
        // SAFETY: this is the first operation in main, before the Tokio runtime
        // or any other process threads exist.
        unsafe {
            std::env::set_var(bmc::manager::SERVICE_NAME_ENV, DEFAULT_SERVICE_NAME);
        }
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<()> {
    log::init();
    let config = cli::Config::parse();
    let system_password = config.system_password.clone();

    let mockfs = mockfs::MockFs::new(&config.mockfs_template, &config.mockfs_path);
    mockfs.init(
        config.mockfs_reset,
        config.factory_default,
        config.setup_pending,
    )?;

    let pacing = config.upgrade_pacing();

    // Shared between the package backend (which requests the stop after a
    // packages-only activation) and the manager (which runs the graceful
    // Axum shutdown when notified).
    let stop = Arc::new(tokio::sync::Notify::new());

    // The registry discovers from a staging tree holding only the widgets that
    // are not still shadowed, so mock-installable widgets stay out of the
    // Add-a-widget list until installed. Installing one re-stages the tree.
    let bundle = config.widgets_path.clone();
    let staging = config.mockfs_path.join("tmp/staged-widgets");
    let scenario_path = mockfs.upgrade_scenario();
    let shadowed: BTreeSet<String> = scenario::read(&scenario_path)
        .shadowed_packages
        .into_iter()
        .collect();
    // A missing bundle root means the mock was started without building the
    // widget bundle (`nix build .#widgets`); stage an empty set and warn rather
    // than aborting. A present-but-unreadable bundle still fails loud, so an
    // install never persists against a silently-empty tree.
    match bundle.try_exists() {
        Ok(true) => widget_staging::stage_installed_widgets(&bundle, &staging, &shadowed)?,
        Ok(false) => {
            warn!(bundle = %bundle.display(),
                "widget bundle not found; starting with no widgets (build it with `nix build .#widgets`)");
            if staging.exists() {
                std::fs::remove_dir_all(&staging)?;
            }
            std::fs::create_dir_all(&staging)?;
        }
        Err(err) => return Err(err.into()),
    }

    let package_backend = Arc::new(
        MockPackageBackend::new(scenario_path, pacing, Arc::clone(&stop))
            .with_package_index(config.package_index.clone())
            .with_widgets_path(Some(bundle))
            .with_staging_path(Some(staging.clone()))
            .with_service_upgrade_marker(mockfs.service_upgrade_marker()),
    );

    let blob = bmc_mock::blob_server::spawn(pacing)
        .await
        .expect("BUG: blob server bind failed");
    let firmware_index = MockIndex::new(mockfs.upgrade_scenario(), blob);

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
        pacing,
        stop,
    );

    let mut config: bmc::Configuration = config.into();
    config.widgets_paths = vec![staging];

    let backlight_driver = MockBacklightDriver::new(true, 18, 20);
    let backlight_driver = Arc::new(tokio::sync::Mutex::new(backlight_driver));

    let led_driver = PlatformLedDriver::new("");

    let compositor = Arc::new(MockCompositor::new(manager.platform().product()));

    bmc::entry::main(
        manager,
        config,
        backlight_driver,
        led_driver.0,
        firmware_index,
        package_backend,
        build_buttons(),
        compositor,
    )
    .await
}

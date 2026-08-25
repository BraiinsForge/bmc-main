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

use std::panic;
use std::path::PathBuf;
use std::{str::FromStr, sync::Arc};

use anyhow::{Result, bail};
use bmc::BmcManager;
use bmc::Configuration;
use bmc::compositor::Compositor;
use bmc_led::apa102_spi::platform_led_driver::PlatformLedDriver;
use bmc_led::disabled::DisabledLedDriver;
use bmc_led::led_driver::LedDriverFactory;
use bmc_openwrt::cli::Parser;
use bmc_openwrt::compositor::EglCompositor;
use bmc_openwrt::{button_driver::UEventButtons, manager::Manager, session::OpenwrtSessionManager};
use bmc_openwrt::{cli::Args, log::build_panic_hook_with_tracing};
use bmc_platform::backlight::{BacklightVisibility, DisplayBacklightDriver};
use bmc_platform::generic_backlight_driver::GenericBacklightDriver;
use bmc_platform::serial_number::BoardSerial;
use bmc_platform::{BmcInfo, BosPlatform, HardwareProfile, HardwareProfileSelection};
use bmc_shared_ii_net_drv::wifi::OpenwrtWifiManager;
use bmc_shared_time::time::Timezone;
use bmc_upgrade::packages::{NixUpgradeConfig, PackageUpgrader};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

fn led_driver_for_profile(
    profile: &bmc_platform::HardwareProfile,
) -> bmc_led::led_driver::LedDriver {
    match profile.led_strip.as_ref() {
        Some(strip) => {
            let device = strip.device.to_string_lossy().into_owned();
            PlatformLedDriver::new(&device).0
        }
        None => DisabledLedDriver::new("/dev/null").0,
    }
}

#[tokio::main]
#[expect(clippy::too_many_lines)]
async fn main() -> Result<()> {
    let args = Args::parse();

    bmc_openwrt::log::init(args.log_to_file);

    panic::set_hook(build_panic_hook_with_tracing());

    let mut config = Configuration {
        widgets_paths: args
            .widgets_paths
            .unwrap_or_else(|| vec![PathBuf::from("/run/current-profile/lib/bmc-widgets")]),
        capture_widget_output: args.log_to_file,
        ..Configuration::default()
    };

    if let Some(address) = args.address {
        config.address = address;
    }
    if let Some(www_path) = args.www_path {
        config.server_config = config
            .server_config
            .set_www_root_path(www_path.clone())
            .set_www_assets_path(www_path.join("assets"))
            .set_www_var_path(www_path.join("var"));
    }

    let bmc_index = bmc::firmware::BmcIndex::default();

    let current_timezone = iana_time_zone::get_timezone()
        .ok()
        .and_then(|timezone| Timezone::from_str(&timezone).ok())
        .unwrap_or_default();

    let platform_override: Option<BosPlatform> =
        match args.hardware_profile.parse::<HardwareProfileSelection>() {
            Ok(selection) => selection.into(),
            Err(err) => {
                error!(%err, "invalid --hardware-profile");
                return Err(err.into());
            }
        };

    let board_serial = match BoardSerial::load_stm32mp157() {
        Ok(serial) => {
            info!("Board serial: {serial}");
            Some(serial)
        }
        Err(err) => {
            warn!(?err, "cannot read the board serial from OTP");
            None
        }
    };

    // BMC_WIFI_SYSPATH overrides the platform path (x86 QEMU with mac80211_hwsim).
    let wifi_path: String = if let Ok(path) = std::env::var("BMC_WIFI_SYSPATH") {
        info!("Using WiFi device path from BMC_WIFI_SYSPATH: {path}");
        path
    } else {
        let platform = match platform_override {
            Some(platform) => platform,
            None => BmcInfo::load()
                .map(|info| info.bmc_platform)
                .inspect_err(|err| {
                    error!(
                        ?err,
                        "cannot resolve platform for WiFi path; pass --hardware-profile"
                    );
                })?,
        };
        if let Some(chip) =
            HardwareProfile::for_product(platform.product()).locate_wifi_chip(board_serial.as_ref())
        {
            info!(?chip, "located WiFi chip");
            chip.syspath().to_string_lossy().into_owned()
        } else {
            info!(?platform, "board carries no WiFi radio");
            String::new()
        }
    };

    info!("Using WiFi device path: {wifi_path}");
    let wifi_manager = Arc::new(
        OpenwrtWifiManager::new(&wifi_path)
            .inspect_err(|err| error!(?err, "Failed to initialize WiFi Manager"))?,
    );

    let manager = Manager::new(
        OpenwrtSessionManager,
        current_timezone,
        wifi_manager,
        platform_override,
        board_serial,
    )
    .await;

    // Has check on factory default already
    if let Err(err) = manager.init_wifi_ap().await {
        error!(?err, "Failed to setup init WiFi AP");
    }

    let profile = bmc_platform::HardwareProfile::for_product(manager.platform().product());
    info!(product = ?profile.product, "resolved hardware profile");

    let led_driver = led_driver_for_profile(&profile);

    let Some(backlight_path) = profile.paths.backlight.as_ref() else {
        bail!(
            "hardware profile {:?} has no backlight path",
            profile.product
        );
    };
    let backlight_path = backlight_path.to_string_lossy().into_owned();
    let mut backlight_driver = GenericBacklightDriver::new(&backlight_path);
    backlight_driver.init()?;
    // The compositor only reads panel visibility, so it gets a driver handle of
    // its own rather than contending for the write lock from the input path.
    let screen_visibility = Arc::new(BacklightVisibility::new(backlight_driver.clone()));
    let backlight_driver = Arc::new(Mutex::new(backlight_driver));

    // Initialize and start the EGL compositor
    let compositor = Arc::new(
        EglCompositor::new(profile.clone(), args.headless_compositor)
            .with_screen_visibility(screen_visibility),
    );
    let wayland_display = compositor
        .start()
        .expect("BUG: failed to start EGL compositor");
    info!("Compositor started on {}", wayland_display);

    let package_backend = Arc::new(PackageUpgrader::new(NixUpgradeConfig {
        servers_config_path: config.nix_servers_config_path.clone(),
        gc_config_path: config.nix_gc_config_path.clone(),
        profile_dir: config.nix_profile_dir.clone(),
        hooks_dir: config.nix_hooks_dir.clone(),
        hooks_override_path: config.nix_hooks_override_path.clone(),
    }));

    bmc::entry::main(
        manager,
        config,
        backlight_driver,
        led_driver,
        bmc_index,
        package_backend,
        Arc::new(Box::new(UEventButtons)),
        compositor,
    )
    .await?;

    Ok(())
}

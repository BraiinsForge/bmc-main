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

use bmc::{Configuration, ServerConfig};
use std::path::{Path, PathBuf};

use clap::Parser;

fn data_dir(subdir: impl AsRef<Path>) -> &'static str {
    let path = dirs::data_local_dir()
        .expect("BUG: cannot determine data_local_dir")
        .join("bmc-mockup")
        .join(subdir)
        .display()
        .to_string();
    Box::leak(path.into_boxed_str())
}

#[derive(Parser, Debug, Clone)]
#[clap(name = "BMC")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "CLI switches are individual bool flags by design"
)]
pub struct Config {
    /// Set server address
    #[clap(long, default_value = "0.0.0.0:6060")]
    pub address: std::net::SocketAddr,
    /// Set path to a web content directory
    #[clap(long, default_value = data_dir("www"))]
    pub www_path: PathBuf,
    /// Override path to a web variable content directory
    #[clap(long)]
    pub www_var_path: Option<PathBuf>,
    /// Set path to a writeable directory for mockup config files
    #[clap(long, default_value = data_dir("mockfs"))]
    pub mockfs_path: PathBuf,
    /// Set path to a directory where the mock files should be copied from
    #[clap(long, default_value = "./bmc-mock/mockfs-template/bmc100")]
    pub mockfs_template: PathBuf,
    /// Delete all local mockfs changes
    #[clap(long)]
    pub mockfs_reset: bool,
    #[clap(long)]
    pub system_password: Option<String>,
    /// Run bmc with a factory-default flag
    #[clap(long)]
    pub factory_default: bool,
    /// BMC config file
    #[clap(long, default_value = "etc/bmc/config.json")]
    pub config_path: PathBuf,
    /// Run bmc with a setup-pending flag
    #[clap(long)]
    pub setup_pending: bool,
    /// Default display brightness. Value between 0-100
    #[clap(long, default_value = "80")]
    pub default_brightness_pct: u8,
    /// Default display brightness in night mode. Value between 0-100
    #[clap(long, default_value = "50")]
    pub default_night_mode_brightness_pct: u8,
    /// Default volume. Value between 0-100
    #[clap(long, default_value = "80")]
    pub default_volume_pct: u8,
    /// Default volume in night mode. Value between 0-100
    #[clap(long, default_value = "50")]
    pub default_night_mode_volume_pct: u8,
    /// Path to a directory with sounds
    #[clap(long, default_value = data_dir("sounds"))]
    pub sounds_dir: PathBuf,
    /// Path to a main crontab file
    #[clap(long, default_value = None)]
    pub crontab_path: Option<PathBuf>,
    /// Path to directory containing widget packages
    #[clap(long, default_value = "./result-widgets")]
    pub widgets_path: PathBuf,

    /// Path to a real nix-package-index.v1.json. When set, the installable
    /// widget catalog is read from it instead of the widget tree under
    /// `--widgets-path`.
    #[clap(long)]
    pub package_index: Option<PathBuf>,

    /// Hardware profile to use: BMC100|BMM100|BMM101|BFM100.
    /// Defaults to `BMC100`.
    #[clap(long = "hardware-profile", default_value = "BMC100")]
    pub hardware_profile: String,

    /// Run simulated upgrades without the realistic delays
    #[clap(long)]
    pub fast_upgrades: bool,
}

impl Config {
    #[must_use]
    pub fn upgrade_pacing(&self) -> crate::pacing::UpgradePacing {
        if self.fast_upgrades {
            crate::pacing::UpgradePacing::Instant
        } else {
            crate::pacing::UpgradePacing::Realistic
        }
    }
}

impl From<Config> for Configuration {
    fn from(value: Config) -> Self {
        let server_config = ServerConfig::default()
            .set_www_root_path(value.www_path.clone())
            .set_www_assets_path(value.www_path.join("assets"))
            .set_www_var_path(
                value
                    .www_var_path
                    .unwrap_or_else(|| value.www_path.join("var")),
            );

        Configuration {
            address: value.address,
            server_config,
            upgrade_image_path: value.mockfs_path.join("tmp/firmware.tar"),
            config_path: value.mockfs_path.join(value.config_path),
            default_brightness_pct: value.default_brightness_pct,
            default_night_mode_brightness_pct: value.default_night_mode_brightness_pct,
            default_volume_pct: value.default_volume_pct,
            default_night_mode_volume_pct: value.default_night_mode_volume_pct,
            sounds_dir: value.sounds_dir,
            crontab_path: value.crontab_path,
            widgets_paths: vec![value.widgets_path],
            capture_widget_output: false,
            nix_servers_config_path: value.mockfs_path.join("etc/nix-upgrade/servers.json"),
            nix_gc_config_path: value.mockfs_path.join("etc/nix-upgrade/gc.json"),
            nix_profile_dir: value.mockfs_path.join("nix/profiles/bmc"),
            pending_install_path: value
                .mockfs_path
                .join("dev/shm/bmc-nix-pending-install.json"),
            nix_hooks_dir: "hooks".to_owned(),
            nix_hooks_override_path: None,
        }
    }
}

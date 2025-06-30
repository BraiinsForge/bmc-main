// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc::{Configuration, ServerConfig};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

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
pub struct Config {
    /// Set server address
    #[clap(long, default_value = "0.0.0.0:6060")]
    pub address: std::net::SocketAddr,
    /// Set gRPC address
    #[clap(long, default_value = "0.0.0.0:50051")]
    pub grpc_address: std::net::SocketAddr,
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
    /// MAC address string for mockup test
    #[clap(long, default_value = "00:0A:35:FF:FF:FF")]
    pub mac_address: String,
    /// IP address string for mockup test
    #[clap(long, default_value = "192.168.0.1")]
    pub ip_address: IpAddr,
    /// Hostname string for mockup test
    #[clap(long, default_value = "bmc-d00627")]
    pub hostname: String,
    /// BMC config file
    #[clap(long, default_value = "etc/bmc_config.json")]
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
            )
            .set_grpc_address(value.grpc_address);

        Configuration {
            address: value.address,
            server_config,
            upgrade_image_path: value.mockfs_path.join("tmp/firmware.tar"),
            config_path: value.mockfs_path.join(value.config_path),
            default_brightness_pct: value.default_brightness_pct,
            default_night_mode_brightness_pct: value.default_night_mode_brightness_pct,
        }
    }
}

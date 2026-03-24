// Copyright (C) 2026  Braiins Systems s.r.o.

mod mock_wifi;
mod virtual_display;

use bmc_nix_init::app::run_app;
use bmc_nix_init::config::InitConfig;
use clap::Parser;
use virtual_display::VirtualDisplayPlatform;

#[derive(Parser)]
#[command(name = "bmc-nix-init-mock")]
struct Cli {
    /// Mock servers.json path
    #[arg(long)]
    servers_config: Option<std::path::PathBuf>,

    /// Simulated BOS version
    #[arg(long, default_value = "26.02")]
    bos_version: String,

    /// Profile directory (uses temp dir if not specified)
    #[arg(long)]
    profile_dir: Option<std::path::PathBuf>,

    /// Simulate no WiFi connection
    #[arg(long)]
    no_wifi: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Cli::parse();

    let tmp_dir = tempfile::tempdir()?;
    let bos_version_path = tmp_dir.path().join("bos_version");
    std::fs::write(&bos_version_path, &args.bos_version)?;

    let config = InitConfig {
        servers_config_path: args
            .servers_config
            .unwrap_or_else(|| std::path::PathBuf::from("servers.json")),
        bos_version_path,
        profile_dir: args
            .profile_dir
            .unwrap_or_else(|| tmp_dir.path().join("profiles")),
        activation_sentinel: tmp_dir.path().join("nix_activated"),
        download_dir: tmp_dir.path().to_path_buf(),
        nix_data_dir: None,
        ..Default::default()
    };

    let platform = VirtualDisplayPlatform::new(1280, 480)?;
    slint::platform::set_platform(Box::new(platform)).expect("BUG: failed to set Slint platform");

    let mock = std::sync::Arc::new(mock_wifi::MockPlatform::new(!args.no_wifi));
    run_app(config, mock);

    Ok(())
}

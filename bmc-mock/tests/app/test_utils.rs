// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use async_trait as _;
use bmc::BmcManager;
use bmc::{App, Configuration};
use bmc_display::{display_driver::DisplayDriver, slint_handle::SlintHandle};
use bmc_led::led_driver::LedDriver;
use bmc_mock::MockSessionManager;
use bmc_mock::{manager::Manager, mock_index::MockIndex, mockfs::MockFs};
use bmc_mock_display::mock_backlight_driver::MockBacklightDriver;
use bmc_platform as _;
use bmc_upgrade::firmware::FirmwareResolver;
use clap as _;
use dirs as _;
use slint as _;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::{env, net::SocketAddr, str::FromStr, sync::Arc};
use tracing as _;
use uuid::Uuid;

const HOSTNAME: &str = "bmc-d00627";
const MAC_ADDRESS: &str = "00:0A:35:FF:FF:FF";
const IP_ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1));

struct TestApp {
    address: String,
}

async fn start_app() -> Result<TestApp> {
    let password = Arc::new(Mutex::new(None));
    let session_manager = MockSessionManager::new(password.clone());

    let random_dir_prefix = Uuid::new_v4().to_string();
    let mockfs_path = env::temp_dir().join(random_dir_prefix);

    let mockfs = MockFs::new(mockfs_path);
    let manager = Manager::new(
        mockfs,
        session_manager,
        password,
        HOSTNAME.to_owned(),
        MAC_ADDRESS.to_owned(),
        IP_ADDRESS,
    );
    let session_manager = manager.session_manager();

    let config = Configuration {
        address: SocketAddr::from_str("127.0.0.1:0").expect("BUG: Cannot bind to socket address"),
        ..Default::default()
    };

    let manager = Arc::new(manager);

    let backlight_driver = MockBacklightDriver::new(true, 10, 20);
    let (slint_handle, _) = SlintHandle::create(0, 0)?;
    let display_driver = DisplayDriver::new(backlight_driver, slint_handle);
    let mut led_driver = LedDriver::new();
    let led_cmd_tx = led_driver.init()?;
    let firmware_resolver = FirmwareResolver::new(MockIndex);

    let app = App::init(
        config,
        manager,
        session_manager,
        display_driver,
        led_driver,
        led_cmd_tx,
        firmware_resolver,
    )
    .await?;

    let port = app.port()?;
    let address = format!("http://localhost:{port}");

    tokio::spawn(app.run());

    Ok(TestApp { address })
}

#[ignore = "Not able to run in CI"]
#[tokio::test]
async fn test_app_is_running() -> Result<()> {
    let app = start_app().await?;

    //This assert will be removed. It's only important that the app is running
    //This is to satisfy clippy as `address` is not used anywhere else
    assert!(!app.address.is_empty());

    Ok(())
}

#[test]
fn always_pass() {
    let always_true = true;
    assert!(always_true);
}

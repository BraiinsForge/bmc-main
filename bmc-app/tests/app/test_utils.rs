// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use std::{net::SocketAddr, str::FromStr, sync::Arc};

use bmc_core::{App, Configuration};

#[cfg(not(target_arch = "arm"))]
use bmc_mock as platform;

#[cfg(target_arch = "arm")]
use bmc_openwrt as platform;

struct TestApp {
    address: String,
}

async fn start_app() -> Result<TestApp> {
    let manager = platform::get_manager();

    let config = Configuration {
        address: SocketAddr::from_str("127.0.0.1:0").expect("Cannot bind to socket address"),
        ..Default::default()
    };

    let manager = Arc::new(manager);

    let app = App::init(config, manager).await?;

    let port = app.port()?;
    let address = format!("http://localhost:{}", port);

    tokio::spawn(app.run());

    Ok(TestApp { address })
}

#[tokio::test]
async fn test_app_is_running() -> Result<()> {
    let app = start_app().await?;

    //This assert will be removed. It's only important that the app is running
    //This is to satisfy clippy as `address` is not used anywhere else
    assert!(!app.address.is_empty());

    Ok(())
}

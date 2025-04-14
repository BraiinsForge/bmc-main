// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc::{App, Configuration};
use bmc_display::display_driver::DisplayHandle;
use bmc_mock::MockManager;
use bmc_mock_display as _;
use clap as _;
use dirs as _;
use slint as _;
use std::{net::SocketAddr, str::FromStr, sync::Arc};

struct TestApp {
    address: String,
}
#[derive(Debug)]
struct MockDisplay;

impl DisplayHandle for MockDisplay {
    fn init(&self) -> anyhow::Result<()> {
        println!("Display initialized");
        Ok(())
    }
}

async fn start_app() -> Result<TestApp> {
    let manager = MockManager {};

    let config = Configuration {
        address: SocketAddr::from_str("127.0.0.1:0").expect("BUG: Cannot bind to socket address"),
        ..Default::default()
    };

    let manager = Arc::new(manager);

    let display = Arc::new(MockDisplay);

    let app = App::init(config, manager, display).await?;

    let port = app.port()?;
    let address = format!("http://localhost:{port}");

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

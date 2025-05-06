// Copyright (C) 2025  Braiins Systems s.r.o.

use std::time::Duration;

use anyhow::Result;
use bmc_display::display_driver::{DisplayHandle, DisplayHandler};
use bmc_mock_display::{
    VirtualDisplay, mock_backlight_driver::MockBacklightDriver,
    mock_data_provider::MockDataProvider,
};
use slint::ComponentHandle;
use tokio as _;
use tracing as _;

#[tokio::main]
async fn main() -> Result<()> {
    let (main_window, display_driver) = VirtualDisplay::create()?;

    let data_provider = MockDataProvider;

    let _timer = display_driver.start_clock_timer(&main_window);

    let display_handler = DisplayHandler::new(display_driver, data_provider);

    display_handler.init()?;

    let scene = Scene::new(display_handler);

    run_scene(scene);

    main_window.run()?;
    Ok(())
}

fn run_scene(_scene: Scene) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // NOTE: Uncomment to run specific sequence of display scenes
        // scene.run_upgrade_failure_scene().await;
    });
}

#[derive(Debug)]
pub struct Scene {
    display_handler: DisplayHandler<MockBacklightDriver, MockDataProvider>,
}

impl Scene {
    const TEN_SEC_DURATION: Duration = Duration::from_secs(10);
    const FIVE_SEC_DURATION: Duration = Duration::from_secs(5);
    #[must_use]
    pub fn new(display_handler: DisplayHandler<MockBacklightDriver, MockDataProvider>) -> Self {
        Self { display_handler }
    }

    pub async fn run_successful_upgrade_scene(&self) {
        self.display_handler
            .emit_event(bmc_display::display_driver::DisplayEvent::DownloadStarted)
            .await;

        tokio::time::sleep(Self::TEN_SEC_DURATION).await;

        self.display_handler
            .emit_event(bmc_display::display_driver::DisplayEvent::UpgradeStarted)
            .await;

        tokio::time::sleep(Self::FIVE_SEC_DURATION).await;

        self.display_handler
            .emit_event(bmc_display::display_driver::DisplayEvent::UpgradeFinishedSuccessfully)
            .await;
    }

    pub async fn run_upgrade_failure_scene(&self) {
        self.display_handler
            .emit_event(bmc_display::display_driver::DisplayEvent::DownloadStarted)
            .await;

        tokio::time::sleep(Self::TEN_SEC_DURATION).await;

        self.display_handler
            .emit_event(bmc_display::display_driver::DisplayEvent::UpgradeStarted)
            .await;

        tokio::time::sleep(Self::FIVE_SEC_DURATION).await;

        self.display_handler
            .emit_event(bmc_display::display_driver::DisplayEvent::UpgradeFailed)
            .await;
    }
}

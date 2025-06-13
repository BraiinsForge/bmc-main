// Copyright (C) 2025  Braiins Systems s.r.o.

#[allow(unused, clippy::allow_attributes)]
// NOTE: slint is not directly used in the code, but defines cargo features
use slint as _;

use anyhow::Result;
use bmc_display::display_controller::DisplayController;
use bmc_display::display_driver::{DisplayHandle, DisplayHandler};
use bmc_mock_display::{VirtualDisplay, mock_data_provider::MockDataProvider};
use std::time::Duration;
use tokio::time::interval;

#[tokio::main]
async fn main() -> Result<()> {
    let (window_handle, display_driver) = VirtualDisplay::create()?;
    spawn_date_time_task(display_driver.display_controller.clone());

    let data_provider = MockDataProvider;

    let display_handler = DisplayHandler::new(display_driver, data_provider);

    let scene = Scene::new(display_handler);

    run_scene(scene);

    window_handle.run()?;
    Ok(())
}

fn run_scene(_scene: Scene) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;

        // NOTE: Uncomment to run specific sequence of display scenes
        // scene.run_upgrade_failure_scene().await;
    });
}

fn spawn_date_time_task(display_controller: DisplayController) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            display_controller.update_datetime();
        }
    });
}

#[derive(Debug)]
pub struct Scene {
    display_handler: DisplayHandler,
}

impl Scene {
    const TEN_SEC_DURATION: Duration = Duration::from_secs(10);
    const FIVE_SEC_DURATION: Duration = Duration::from_secs(5);
    #[must_use]
    pub fn new(display_handler: DisplayHandler) -> Self {
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

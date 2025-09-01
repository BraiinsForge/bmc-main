// Copyright (C) 2025  Braiins Systems s.r.o.

#[allow(unused, clippy::allow_attributes)]
// NOTE: slint is not directly used in the code, but defines cargo features
use slint as _;

use anyhow::Result;
use bmc_display::data::Screen;
use bmc_display::display_controller::DisplayController;
use bmc_mock_display::VirtualDisplay;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    let (window_handle, display_driver) = VirtualDisplay::create()?;

    let scene = Scene::new(display_driver.display_controller);

    run_scene(scene);

    window_handle.run()?;
    Ok(())
}

fn run_scene(_scene: Scene) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(1)).await;

        // NOTE: Uncomment to run specific sequence of display scenes
        // scene.run_upgrade_failure_scene().await;
    });
}

#[derive(Debug)]
pub struct Scene {
    display_controller: DisplayController,
}

impl Scene {
    const FIVE_SEC_DURATION: Duration = Duration::from_secs(5);
    #[must_use]
    pub fn new(display_controller: DisplayController) -> Self {
        Self { display_controller }
    }

    pub async fn run_successful_upgrade_scene(&self) {
        self.display_controller.set_screen(Screen::DownloadFirmware);
        self.simulate_download_progress().await;
        self.display_controller.set_screen(Screen::Upgrade);
        sleep(Self::FIVE_SEC_DURATION).await;
        self.display_controller.set_screen(Screen::UpgradeSuccess);
    }

    pub async fn run_upgrade_failure_scene(&self) {
        self.display_controller.set_screen(Screen::DownloadFirmware);
        self.simulate_download_progress().await;
        self.display_controller.set_screen(Screen::Upgrade);
        sleep(Self::FIVE_SEC_DURATION).await;
        self.display_controller.set_screen(Screen::UpgradeFailed);
    }

    async fn simulate_download_progress(&self) {
        const FILE_SIZE: f32 = 41.2;
        const NUMBER_OF_UPDATES: i32 = 30;

        let total = FILE_SIZE;
        #[expect(clippy::cast_precision_loss)]
        let step = total / NUMBER_OF_UPDATES as f32;

        for i in 1..=NUMBER_OF_UPDATES {
            #[expect(clippy::cast_precision_loss)]
            let downloaded = step * i as f32;

            self.display_controller
                .update_download_firmware_progress(downloaded, total);

            sleep(Duration::from_millis(300)).await;
        }
    }
}

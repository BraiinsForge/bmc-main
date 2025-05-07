// Copyright (C) 2025  Braiins Systems s.r.o.

use std::time::Duration;

use bmc_display::data_provider::DownloadProgress;
use tokio::sync::mpsc::unbounded_channel;

const FILE_SIZE: f32 = 41.2;
const NUMBER_OF_UPDATES: i32 = 30;

#[derive(Clone, Debug)]
pub struct MockDataProvider;

impl bmc_display::data_provider::DataProvider for MockDataProvider {
    fn get_download_firmware_screen_data(
        &self,
    ) -> bmc_display::data_provider::DownloadFirmwareScreenData {
        let (tx, rx) = unbounded_channel();

        tokio::spawn(async move {
            let total = FILE_SIZE;
            #[expect(clippy::cast_precision_loss)]
            let step = total / NUMBER_OF_UPDATES as f32;

            for i in 1..=NUMBER_OF_UPDATES {
                #[expect(clippy::cast_precision_loss)]
                let downloaded = step * i as f32;
                let progress = DownloadProgress {
                    downloaded_mb: downloaded,
                    total_mb: total,
                };
                tx.send(progress)
                    .expect("BUG: cannot send download progress to display channel");

                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        });

        bmc_display::data_provider::DownloadFirmwareScreenData {
            progress_receiver: rx,
        }
    }
}

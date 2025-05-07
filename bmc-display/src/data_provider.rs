// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;

pub trait DataProvider: Send + Sync + Clone + Debug + 'static {
    fn get_download_firmware_screen_data(&self) -> DownloadFirmwareScreenData;
}

#[derive(Clone, Debug)]
pub struct DownloadProgress {
    pub downloaded_mb: f32,
    pub total_mb: f32,
}

#[derive(Debug)]
pub struct DownloadFirmwareScreenData {
    pub progress_receiver: tokio::sync::mpsc::UnboundedReceiver<DownloadProgress>,
}

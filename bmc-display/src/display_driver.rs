// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::{Screen, Widget};
use std::{
    fmt::Debug,
    fs::File,
    path::Path,
    sync::{Arc, Mutex},
};
use tracing::info;

use tokio::sync::mpsc::Sender;

const EVENT_BUFFER_SIZE: usize = 1024;

use crate::{data_provider::DataProvider, display_controller::DisplayController};

pub trait DisplayBacklightDriver: Sync + Send + Clone + Debug + 'static {
    fn init(&mut self) -> anyhow::Result<()>;

    fn change_state(&self, enabled: bool) -> anyhow::Result<()>;

    fn state(&self) -> anyhow::Result<bool>;

    fn toggle_state(&mut self) -> anyhow::Result<()> {
        self.state().and_then(|state| self.change_state(!state))
    }

    fn turn_on(&self) -> anyhow::Result<()> {
        self.change_state(true)
    }

    fn turn_off(&self) -> anyhow::Result<()> {
        self.change_state(false)
    }

    fn brightness(&self) -> anyhow::Result<u8>;

    fn max_brightness(&self) -> u8;

    fn set_brightness(&self, value: u8) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait DisplayHandle: Sync + Send + Clone + Debug {
    async fn emit_event(&self, event: DisplayEvent);
}

#[derive(Debug)]
pub struct DisplayDriver<T: DisplayBacklightDriver> {
    pub backlight_driver: Arc<Mutex<T>>,
    pub display_controller: DisplayController,
}

impl<T: DisplayBacklightDriver> DisplayDriver<T> {
    pub fn init(
        backlight_driver: T,
        display_controller: DisplayController,
    ) -> anyhow::Result<Self> {
        let json_data = Path::new("widgets.json");
        let widgets: Vec<Widget> = File::open(json_data)
            .map_err(|e| {
                println!("Cannot open file {json_data:?}: {e}");
            })
            .and_then(|file| {
                serde_json::from_reader(file).map_err(|e| {
                    println!("Cannot read widget data: {e}");
                })
            })
            .unwrap_or_default();

        display_controller.populate_widgets(widgets);
        backlight_driver.turn_on()?;

        Ok(Self {
            backlight_driver: Arc::new(Mutex::new(backlight_driver)),
            display_controller,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DisplayHandler {
    event_sender: Sender<DisplayEvent>,
}

impl DisplayHandler {
    pub fn new<T: DisplayBacklightDriver, U: DataProvider>(
        display_driver: DisplayDriver<T>,
        data_provider: U,
    ) -> Self {
        Self {
            event_sender: EventHandler::init(data_provider, display_driver.display_controller),
        }
    }
}

#[async_trait::async_trait]
impl DisplayHandle for DisplayHandler {
    async fn emit_event(&self, event: DisplayEvent) {
        _ = self.event_sender.send(event).await;
    }
}

#[derive(Debug)]
pub enum DisplayEvent {
    DownloadStarted,
    UpgradeStarted,
    UpgradeFailed,
    UpgradeFinishedSuccessfully,
    TimezoneChanged,
}

struct EventHandler;

impl EventHandler {
    fn init<T: DataProvider>(
        data_provider: T,
        display_controller: DisplayController,
    ) -> Sender<DisplayEvent> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_BUFFER_SIZE);

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                match event {
                    DisplayEvent::DownloadStarted => {
                        Self::handle_upgrade_progress(
                            data_provider.clone(),
                            display_controller.clone(),
                        );
                    }
                    DisplayEvent::UpgradeStarted => {
                        Self::set_screen(Screen::Upgrade, &display_controller);
                    }
                    DisplayEvent::UpgradeFailed => {
                        Self::set_screen(Screen::UpgradeFailed, &display_controller);
                    }
                    DisplayEvent::UpgradeFinishedSuccessfully => {
                        Self::set_screen(Screen::UpgradeSuccess, &display_controller);
                    }
                    DisplayEvent::TimezoneChanged => {
                        info!("Timezone was changed");
                    }
                }
            }
        });

        sender
    }

    fn handle_upgrade_progress<T: DataProvider>(
        data_provider: T,
        display_controller: DisplayController,
    ) {
        Self::set_screen(Screen::DownloadFirmware, &display_controller);
        tokio::spawn(async move {
            let mut screen_data = data_provider.get_download_firmware_screen_data();

            while let Some(data) = screen_data.progress_receiver.recv().await {
                display_controller
                    .update_download_firmware_progress(data.downloaded_mb, data.total_mb);
            }
        });
    }

    fn set_screen(screen: Screen, display_controller: &DisplayController) {
        display_controller.set_screen(screen);
    }
}

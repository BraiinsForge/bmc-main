// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{
    data::{Screen, Widget},
    generated::{DateTimeAdapter, MainWindow},
};
use chrono::{Datelike, Timelike};
use slint::{ComponentHandle, Global, Timer};
use std::time::Duration;
use std::{
    fmt::Debug,
    fs::File,
    path::Path,
    sync::{Arc, Mutex},
};
use tracing::info;

use tokio::sync::mpsc::Sender;

const EVENT_BUFFER_SIZE: usize = 1024;

use crate::{data_provider::DataProvider, slint_handle::SlintHandle};

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
    fn init(&self) -> anyhow::Result<()>;

    async fn emit_event(&self, event: DisplayEvent);
}

#[derive(Debug)]
pub struct DisplayDriver<T: DisplayBacklightDriver> {
    backlight_driver: Arc<Mutex<T>>,
    slint_handle: SlintHandle,
}

impl<T: DisplayBacklightDriver> DisplayDriver<T> {
    pub fn new(backlight_driver: T, slint_handle: SlintHandle) -> Self {
        Self {
            backlight_driver: Arc::new(Mutex::new(backlight_driver)),
            slint_handle,
        }
    }

    #[must_use]
    pub fn start_clock_timer(&self, window: &MainWindow) -> Timer {
        let timer = Timer::default();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), {
            let window_weak = window.as_weak();
            move || {
                let _ = window_weak.upgrade_in_event_loop(move |main_window| {
                    let datetime_adapter = DateTimeAdapter::get(&main_window);
                    let now = chrono::Local::now();
                    datetime_adapter.set_hour24(i32::try_from(now.hour()).unwrap_or_default());
                    datetime_adapter.set_hour12(i32::try_from(now.hour12().1).unwrap_or_default());
                    datetime_adapter.set_is_pm(now.hour12().0);
                    datetime_adapter.set_minute(i32::try_from(now.minute()).unwrap_or_default());
                    datetime_adapter.set_second(i32::try_from(now.second()).unwrap_or_default());
                    datetime_adapter.set_day(i32::try_from(now.day()).unwrap_or_default());
                    datetime_adapter.set_month(i32::try_from(now.month()).unwrap_or_default());
                    datetime_adapter.set_year(now.year());
                    datetime_adapter.set_weekday(slint::format!("{}", now.weekday()));
                });
            }
        });
        timer
    }
}

#[derive(Debug, Clone)]
pub struct DisplayHandler<T: DisplayBacklightDriver, U: DataProvider> {
    backlight_driver: Arc<Mutex<T>>,
    slint_handle: SlintHandle,
    event_sender: Sender<DisplayEvent>,
    _data_provider: U,
}

impl<T: DisplayBacklightDriver, U: DataProvider> DisplayHandler<T, U> {
    pub fn new(display_driver: DisplayDriver<T>, data_provider: U) -> Self {
        Self {
            backlight_driver: display_driver.backlight_driver.clone(),
            slint_handle: display_driver.slint_handle.clone(),
            event_sender: EventHandler::init(data_provider.clone(), display_driver.slint_handle),
            _data_provider: data_provider,
        }
    }
}

#[async_trait::async_trait]
impl<T: DisplayBacklightDriver, U: DataProvider> DisplayHandle for DisplayHandler<T, U> {
    fn init(&self) -> anyhow::Result<()> {
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

        let _ = self.slint_handle.populate_widgets(widgets);

        self.backlight_driver
            .lock()
            .expect("BUG: cannot lock display")
            .turn_on()?;

        Ok(())
    }

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
    fn init<T: DataProvider>(data_provider: T, slint_handle: SlintHandle) -> Sender<DisplayEvent> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_BUFFER_SIZE);

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                match event {
                    DisplayEvent::DownloadStarted => {
                        Self::handle_upgrade_progress(data_provider.clone(), slint_handle.clone());
                    }
                    DisplayEvent::UpgradeStarted => {
                        Self::set_screen(Screen::Upgrade, &slint_handle);
                    }
                    DisplayEvent::UpgradeFailed => {
                        Self::set_screen(Screen::UpgradeFailed, &slint_handle);
                    }
                    DisplayEvent::UpgradeFinishedSuccessfully => {
                        Self::set_screen(Screen::UpgradeSuccess, &slint_handle);
                    }
                    DisplayEvent::TimezoneChanged => {
                        info!("Timezone was changed");
                    }
                }
            }
        });

        sender
    }

    fn handle_upgrade_progress<T: DataProvider>(data_provider: T, slint_handle: SlintHandle) {
        Self::set_screen(Screen::DownloadFirmware, &slint_handle);
        tokio::spawn(async move {
            let mut screen_data = data_provider.get_download_firmware_screen_data();

            while let Some(data) = screen_data.progress_receiver.recv().await {
                _ = slint_handle
                    .update_download_firmware_progress(data.downloaded_mb, data.total_mb);
            }
        });
    }

    fn set_screen(screen: Screen, slint_handle: &SlintHandle) {
        _ = slint_handle.set_screen(screen);
    }
}

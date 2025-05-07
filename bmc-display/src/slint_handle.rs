// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::{Screen, WidgetType};
use crate::generated::{
    Backend, ClockLarge, ClockMedium, ClockSmall, MainWindow, UpgradeDownloadAdapter,
};
use anyhow::anyhow;
use chrono::{Duration, Utc};
use core::fmt;
use slint::{ComponentHandle, Global, ModelRc, VecModel, Weak};
use std::sync::{Arc, Mutex};

fn into_populate_widgets_closure(
    widgets: Vec<WidgetType>,
) -> Box<dyn FnOnce(MainWindow) + Send + 'static> {
    Box::new(move |main_window: MainWindow| {
        let mut clock_small_vec: Vec<ClockSmall> = vec![];
        let mut clock_medium_vec: Vec<ClockMedium> = vec![];
        let mut clock_large_vec: Vec<ClockLarge> = vec![];
        for widget in widgets {
            match widget {
                WidgetType::ClockSmall(clock_small) => {
                    clock_small_vec.push(clock_small);
                }
                WidgetType::ClockMedium(clock_medium) => {
                    clock_medium_vec.push(clock_medium);
                }
                WidgetType::ClockLarge(clock_large) => {
                    clock_large_vec.push(clock_large);
                }
            }
        }
        main_window.set_clock_small(ModelRc::new(VecModel::from(clock_small_vec)));
        main_window.set_clock_medium(ModelRc::new(VecModel::from(clock_medium_vec)));
        main_window.set_clock_large(ModelRc::new(VecModel::from(clock_large_vec)));
    })
}

#[derive(Clone)]
pub struct SlintHandle {
    ui_handle: Arc<Mutex<Weak<MainWindow>>>,
}

impl SlintHandle {
    pub fn create(width: u32, height: u32) -> anyhow::Result<(Self, MainWindow)> {
        let main_window = MainWindow::new()?;
        main_window
            .window()
            .set_size(slint::PhysicalSize::new(width, height));

        Ok((
            Self {
                ui_handle: Arc::new(Mutex::new(main_window.as_weak())),
            },
            main_window,
        ))
    }

    pub fn update_in_event_loop<F>(&self, f: F) -> anyhow::Result<()>
    where
        F: FnOnce(MainWindow) + Send + 'static,
    {
        self.ui_handle
            .lock()
            .expect("BUG: cannot lock ui_handle")
            .upgrade_in_event_loop(f)
            .map_err(|e| anyhow!("Cannot upgrade ui_handle: {:?}", e))
    }

    pub fn populate_widgets(&self, widgets: Vec<WidgetType>) -> anyhow::Result<()> {
        self.update_in_event_loop(into_populate_widgets_closure(widgets))
    }

    pub fn init_ui(&self) -> anyhow::Result<()> {
        self.update_in_event_loop(move |main_window| {
            // Dummy implementation of timezone backend logic
            main_window.global::<Backend<'_>>().on_get_time(|city| {
                let offset: i64 = match city.as_str() {
                    "NYC" => -4,
                    "LON" => 1,
                    "PRG" => 2,
                    "HKG" => 8,
                    "TOK" => 9,
                    _ => 0,
                };
                let now = Utc::now();
                slint::format!("{}", (now + Duration::hours(offset)).format("%H:%M:%S"))
            });
        })
    }

    pub fn update_download_firmware_progress(
        &self,
        downloaded_mb: f32,
        total_mb: f32,
    ) -> anyhow::Result<()> {
        self.update_in_event_loop(into_update_download_progress_closure(
            downloaded_mb,
            total_mb,
        ))
    }

    pub fn set_screen(&self, screen: Screen) -> anyhow::Result<()> {
        self.update_in_event_loop(Box::new(move |main_window: MainWindow| {
            main_window.set_screen_id(screen.into());
        }))
    }
}

impl fmt::Debug for SlintHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlintHandle")
            .field("ui_handle", &"Weak<MainWindow>")
            .finish()
    }
}

fn into_update_download_progress_closure(
    downloaded_mb: f32,
    total_mb: f32,
) -> Box<dyn FnOnce(MainWindow) + Send + 'static> {
    let mut progress = 0.0;
    if total_mb > 0.0 {
        progress = downloaded_mb / total_mb;
    }

    Box::new(move |main_window: MainWindow| {
        let upgrade_download_adapter = UpgradeDownloadAdapter::get(&main_window);

        upgrade_download_adapter.set_progress(progress);
        upgrade_download_adapter.set_downloaded_mb_text(slint::SharedString::from(format!(
            "{} MB of {} MB",
            round_to_one_decimal(downloaded_mb),
            round_to_one_decimal(total_mb)
        )));
        upgrade_download_adapter.set_progress_text(slint::SharedString::from(format!(
            "Downloading firmware {}%...",
            (progress * 100.0).round(),
        )));
    })
}

fn round_to_one_decimal(value: f32) -> f32 {
    (value * 10.0).round() / 10.0
}

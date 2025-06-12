// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::{Screen, Widget};
use crate::generated::{MainWindow, UpgradeDownloadAdapter, WidgetSlint};
use anyhow::anyhow;
use core::fmt;
use slint::{ComponentHandle, Global, ModelRc, SharedString, VecModel, Weak};
use std::sync::{Arc, Mutex};

fn into_populate_widgets_closure(
    widgets: Vec<Widget>,
) -> Box<dyn FnOnce(MainWindow) + Send + 'static> {
    Box::new(move |main_window: MainWindow| {
        let mut widget_slint: Vec<WidgetSlint> = vec![];
        for widget in widgets {
            widget_slint.push(WidgetSlint {
                col: widget.col,
                row: widget.row,
                widget_data: ModelRc::new(VecModel::from(
                    widget
                        .widget_data
                        .iter()
                        .map(std::convert::Into::into)
                        .collect::<Vec<SharedString>>(),
                )),
                widget_size: widget.widget_size,
                widget_type: widget.widget_type,
            });
        }
        main_window.set_widgets(ModelRc::new(VecModel::from(widget_slint)));
    })
}

#[derive(Clone)]
pub struct DisplayController {
    ui_handle: Arc<Mutex<Weak<MainWindow>>>,
}

impl DisplayController {
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

    pub fn populate_widgets(&self, widgets: Vec<Widget>) -> anyhow::Result<()> {
        self.update_in_event_loop(into_populate_widgets_closure(widgets))
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

impl fmt::Debug for DisplayController {
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

// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::WidgetType;
use anyhow::anyhow;
use chrono::{Duration, Utc};
use core::fmt;
use slint::{ComponentHandle, ModelRc, VecModel, Weak};
use std::sync::{Arc, Mutex};

use crate::generated::{Backend, ClockLarge, ClockMedium, ClockSmall, MainWindow};

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
}

impl fmt::Debug for SlintHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlintHandle")
            .field("ui_handle", &"Weak<MainWindow>")
            .finish()
    }
}

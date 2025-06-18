// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::{Scene, Screen};
use crate::display_controller::DisplayController;
use crate::generated::DateTimeAdapter;
use crate::generated::{self, WidgetSlint};
use chrono::{Datelike, Timelike};
use slint::{Global, Model, ModelRc, VecModel};

impl DisplayController {
    #[expect(unused)]
    #[track_caller]
    /// Use this function to cast ModelRc<T> into VecModel<T> when you want to manipulate items in the VecModel
    fn vec_model_ref<T: 'static>(model_rc: &ModelRc<T>) -> &VecModel<T> {
        model_rc
            .as_any()
            .downcast_ref::<VecModel<T>>()
            .expect("BUG: failed to extract VecModel")
    }

    pub fn populate_widgets(&self, scenes: Vec<Scene>) {
        self.in_event_loop(move |main_window| {
            let mut widget_slint: Vec<WidgetSlint> = vec![];
            for scene in scenes {
                for widget in scene.widgets {
                    widget_slint.push(widget.into());
                }
            }
            main_window.set_widgets(ModelRc::new(VecModel::from(widget_slint)));
        });
    }

    pub fn update_datetime(&self) {
        self.in_event_loop(|main_window| {
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

    pub fn update_download_firmware_progress(&self, downloaded_mb: f32, total_mb: f32) {
        fn round_to_one_decimal(value: f32) -> f32 {
            (value * 10.0).round() / 10.0
        }

        self.in_event_loop(move |main_window| {
            let mut progress = 0.0;
            if total_mb > 0.0 {
                progress = downloaded_mb / total_mb;
            }

            let upgrade_download_adapter = generated::UpgradeDownloadAdapter::get(&main_window);

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
        });
    }

    pub fn set_screen(&self, screen: Screen) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            main_window.set_screen_id(screen.into());
        });
    }
}

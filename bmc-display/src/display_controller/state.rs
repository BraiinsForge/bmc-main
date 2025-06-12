// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::{Screen, Widget};
use crate::display_controller::DisplayController;
use crate::generated;
use slint::{Global, Model, ModelRc, SharedString, VecModel};

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

    pub fn populate_widgets(&self, widgets: Vec<Widget>) {
        self.in_event_loop(move |main_window| {
            let mut widget_slint: Vec<generated::WidgetSlint> = vec![];
            for widget in widgets {
                widget_slint.push(generated::WidgetSlint {
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

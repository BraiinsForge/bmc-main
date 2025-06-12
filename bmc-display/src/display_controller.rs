// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::{Screen, Widget};
use crate::generated;
use core::fmt;
use slint::{ComponentHandle, Global, ModelRc, SharedString, VecModel, Weak};
use tracing::debug;

#[derive(Clone)]
pub struct DisplayController {
    main_window_weak: Weak<generated::MainWindow>,
}

impl DisplayController {
    pub fn create(width: u32, height: u32) -> anyhow::Result<(Self, WindowHandle)> {
        let main_window = generated::MainWindow::new()?;
        main_window
            .window()
            .set_size(slint::PhysicalSize::new(width, height));

        let controller = Self {
            main_window_weak: main_window.as_weak(),
        };

        Ok((controller, WindowHandle(main_window)))
    }

    fn in_event_loop<F>(&self, func: F)
    where
        F: FnOnce(generated::MainWindow) + Send + 'static,
    {
        if let Err(err) = self.main_window_weak.upgrade_in_event_loop(func) {
            debug!(?err, "failed to run function in slint event loop");
        }
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

impl fmt::Debug for DisplayController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DisplayController").finish()
    }
}

pub struct WindowHandle(generated::MainWindow);

impl WindowHandle {
    // TODO: remove whole function
    #[must_use]
    pub fn slint_main_window(&self) -> &generated::MainWindow {
        &self.0
    }

    pub fn run(self) -> anyhow::Result<()> {
        Ok(self.0.run()?)
    }
}

impl fmt::Debug for WindowHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowHandle").finish()
    }
}

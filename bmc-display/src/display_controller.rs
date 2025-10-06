// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod callback;
pub mod state;

use crate::generated;
use core::fmt;
use slint::{CloseRequestResponse, ComponentHandle, Weak};
use std::sync::Arc;
use tokio::sync::Notify;
use tokio::sync::futures::OwnedNotified;
use tracing::debug;

#[derive(Clone)]
pub struct DisplayController {
    main_window_weak: Weak<generated::MainWindow>,
    window_closed: Arc<Notify>,
}

impl DisplayController {
    pub fn create(width: u32, height: u32) -> anyhow::Result<(Self, WindowHandle)> {
        let main_window = generated::MainWindow::new()?;
        main_window
            .window()
            .set_size(slint::PhysicalSize::new(width, height));

        let window_closed = Arc::new(Notify::new());

        main_window.window().on_close_requested({
            let window_closed = window_closed.clone();
            move || {
                window_closed.notify_waiters();
                CloseRequestResponse::HideWindow
            }
        });

        let controller = Self {
            main_window_weak: main_window.as_weak(),
            window_closed,
        };

        Self::setup_static_callbacks(&main_window);

        Ok((controller, WindowHandle(main_window)))
    }

    pub fn window_closed(&self) -> OwnedNotified {
        self.window_closed.clone().notified_owned()
    }

    pub fn quit(&self) {
        if let Err(err) = slint::quit_event_loop() {
            debug!(?err, "failed to quit slint event loop");
        }
    }

    fn in_event_loop<F>(&self, func: F)
    where
        F: FnOnce(generated::MainWindow) + Send + 'static,
    {
        if let Err(err) = self.main_window_weak.upgrade_in_event_loop(func) {
            debug!(?err, "failed to run function in slint event loop");
        }
    }
}

impl fmt::Debug for DisplayController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DisplayController").finish()
    }
}

pub struct WindowHandle(generated::MainWindow);

impl WindowHandle {
    pub fn run(self) -> anyhow::Result<()> {
        Ok(self.0.run()?)
    }
}

impl fmt::Debug for WindowHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowHandle").finish()
    }
}

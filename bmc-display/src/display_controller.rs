// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod callback;
pub mod state;

use crate::generated;
use core::fmt;
use slint::{ComponentHandle, Weak};
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

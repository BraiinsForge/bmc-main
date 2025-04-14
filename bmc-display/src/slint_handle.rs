// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::anyhow;
use core::fmt;
use slint::Weak;
use std::sync::{Arc, Mutex};

use crate::generated::*;

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
}

impl fmt::Debug for SlintHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlintHandle")
            .field("ui_handle", &"Weak<MainWindow>")
            .finish()
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};

use crate::slint_handle::SlintHandle;

pub trait DisplayBacklightDriver: Sync + Send + Debug {
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

pub trait DisplayHandle: Sync + Send + Debug {
    fn init(&self) -> anyhow::Result<()>;
}

#[derive(Debug)]
pub struct DisplayDriver {
    backlight_driver: Arc<Mutex<dyn DisplayBacklightDriver>>,
    slint_handle: SlintHandle,
}

impl DisplayDriver {
    pub fn new<T: DisplayBacklightDriver + 'static>(
        backlight_driver: T,
        slint_handle: SlintHandle,
    ) -> Self {
        Self {
            backlight_driver: Arc::new(Mutex::new(backlight_driver)),
            slint_handle,
        }
    }
}

impl DisplayHandle for DisplayDriver {
    fn init(&self) -> anyhow::Result<()> {
        // TODO: this is to prevent clippy to fail. Slint handle isn't used at this moment
        let _ = self.slint_handle;

        self.backlight_driver
            .lock()
            .expect("BUG: cannot lock display")
            .turn_on()
    }
}

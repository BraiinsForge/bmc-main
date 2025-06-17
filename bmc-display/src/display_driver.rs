// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::display_controller::DisplayController;
use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};

pub trait DisplayBacklightDriver: Sync + Send + Clone + Debug + 'static {
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

#[derive(Debug)]
pub struct DisplayDriver<T: DisplayBacklightDriver> {
    pub backlight_driver: Arc<Mutex<T>>,
    pub display_controller: DisplayController,
}

impl<T: DisplayBacklightDriver> DisplayDriver<T> {
    pub fn init(
        backlight_driver: T,
        display_controller: DisplayController,
    ) -> anyhow::Result<Self> {
        backlight_driver.turn_on()?;

        Ok(Self {
            backlight_driver: Arc::new(Mutex::new(backlight_driver)),
            display_controller,
        })
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;

/// Trait for controlling display backlight hardware.
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

    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::integer_division)]
    fn pct_to_brightness(&self, percent: u8) -> u8 {
        ((u16::from(percent) * u16::from(self.max_brightness())) / 100) as u8
    }

    fn set_brightness_pct(&self, percent: u8) -> anyhow::Result<()> {
        self.set_brightness(self.pct_to_brightness(percent))
    }
}

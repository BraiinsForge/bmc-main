// Copyright (C) 2025  Braiins Systems s.r.o.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootloaderConfig {
    /// Night mode start time in UTC minutes. None means night mode is disabled.
    pub night_from_utc_minutes: Option<u16>,
    /// Night mode end time in UTC minutes. None means night mode is disabled.
    pub night_to_utc_minutes: Option<u16>,
    /// LED enabled during day
    pub led_day: bool,
    /// LED enabled during night. None means night mode is disabled.
    pub led_night: Option<bool>,
    /// Screen brightness during day (actual hardware value, not percentage)
    pub screen_day: u8,
    /// Screen brightness during night (actual hardware value, not percentage). None means night mode is disabled.
    pub screen_night: Option<u8>,
}

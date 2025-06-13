// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::mock_backlight_driver::MockBacklightDriver;
use anyhow::{Context, Result};
use bmc_display::{
    display_controller::{DisplayController, WindowHandle},
    display_driver::DisplayDriver,
    metadata::{DisplayMetadata, ResolutionMetadata, UsizeMetadata},
};

#[derive(Debug)]
pub struct VirtualDisplay;

impl VirtualDisplay {
    pub fn create() -> Result<(WindowHandle, DisplayDriver<MockBacklightDriver>)> {
        let brightness = UsizeMetadata::new(18, 0, 20);
        let resolution = ResolutionMetadata::new(1200, 400);
        let display_metadata = DisplayMetadata::new(brightness, resolution);

        let (display_controller, main_window) = DisplayController::create(
            display_metadata.resolution.width,
            display_metadata.resolution.height,
        )
        .context("Cannot initialize ui")?;

        let backlight_driver = MockBacklightDriver::new(
            false,
            u8::try_from(display_metadata.brightness.default)?,
            u8::try_from(display_metadata.brightness.max)?,
        );

        let display_driver = DisplayDriver::init(backlight_driver, display_controller)?;

        Ok((main_window, display_driver))
    }
}

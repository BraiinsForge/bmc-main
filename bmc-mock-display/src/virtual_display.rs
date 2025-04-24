// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::mock_backlight_driver::MockBacklightDriver;
use anyhow::{Context, Result};
use bmc_display::{
    display_driver::DisplayDriver,
    generated::MainWindow,
    metadata::{DisplayMetadata, ResolutionMetadata, UsizeMetadata},
    slint_handle::SlintHandle,
};

#[derive(Debug)]
pub struct VirtualDisplay;

impl VirtualDisplay {
    pub fn create() -> Result<(MainWindow, DisplayDriver)> {
        let brightness = UsizeMetadata::new(18, 0, 20);
        let resolution = ResolutionMetadata::new(1200, 400);
        let display_metadata = DisplayMetadata::new(brightness, resolution);

        let (slint_handle, main_window) = SlintHandle::create(
            display_metadata.resolution.width,
            display_metadata.resolution.height,
        )
        .context("Cannot initialize ui")?;

        let backlight_driver = MockBacklightDriver::new(
            false,
            u8::try_from(display_metadata.brightness.default)?,
            u8::try_from(display_metadata.brightness.max)?,
        );

        let display_driver = DisplayDriver::new(backlight_driver, slint_handle);

        Ok((main_window, display_driver))
    }
}

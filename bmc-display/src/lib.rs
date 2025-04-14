// Copyright (C) 2025  Braiins Systems s.r.o.

#[allow(warnings)]
pub(crate) mod generated {
    slint::include_modules!();
}
pub mod display_driver;
pub mod metadata;
pub mod proxy;
pub mod slint_handle;

#[cfg(feature = "standalone")]
pub mod mock_backlight_driver;
#[cfg(feature = "standalone")]
pub mod virtual_display;

// Copyright (C) 2025  Braiins Systems s.r.o.

mod params;
mod widget;
pub mod widget_protocol;

pub use params::Config;
pub use widget::DigitalClockWidget;

#[allow(warnings)]
mod generated {
    slint::include_modules!();
}
pub use generated::*;

// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod ipc;
mod params;
mod widget;

pub use params::{Config, Params};
pub use widget::DigitalClockWidget;

#[allow(warnings)]
mod generated {
    slint::include_modules!();
}
pub use generated::*;

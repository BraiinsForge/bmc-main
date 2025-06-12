// Copyright (C) 2025  Braiins Systems s.r.o.

#[allow(warnings)]
pub mod generated {
    slint::include_modules!();
}
pub mod data;
pub mod data_provider;
pub mod display_controller;
pub mod display_driver;
pub mod metadata;
pub mod proxy;

use tracing as _;

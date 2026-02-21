// Copyright (C) 2025  Braiins Systems s.r.o.

#[allow(warnings)]
mod generated {
    slint::include_modules!();
}
pub mod bitcoin_data;
pub mod blockheight_data;
pub mod btc_history_data;
pub mod clock_data;
pub mod data;
pub mod diff_hashrate_data;
pub mod difficulty_data;
pub mod display_controller;
pub mod display_driver;
pub mod graph_utils;
pub mod halving_data;
pub mod hashrate_data;
mod indexmap_model;
pub mod metadata;
pub mod pool_data;
pub mod proxy;
pub mod remote_image_data;
pub mod remote_widget_data;
mod utils;

pub use slint::{
    Rgb8Pixel, Rgba8Pixel, SharedPixelBuffer, private_unstable_api::re_exports::SharedImageBuffer,
};

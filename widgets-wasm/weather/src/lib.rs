// Copyright (C) 2026  Braiins Systems s.r.o.

//! Weather widget — current conditions and forecast, four sizes.
//! Ported from `deckfeeder/assets/widgets/weather/` (a JS/HTML widget).

mod manifest_params;
pub mod model;
pub mod weather_code;
pub mod wind;

#[unsafe(no_mangle)]
pub extern "C" fn init() {}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {}

// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::Rgb;

pub const LED_COUNT: u8 = 10;
pub const LED_MAX_BRIGHTNESS: f32 = 1.0;

pub const LED_COLOR_RED: Rgb = Rgb::new(0xFF, 0x00, 0x00);
pub const LED_COLOR_MAGENTA: Rgb = Rgb::new(0xFF, 0x00, 0xFF);
pub const LED_COLOR_CYAN: Rgb = Rgb::new(0x00, 0xFF, 0xFF);
pub const LED_COLOR_WARM_WHITE: Rgb = Rgb::new(0xF6, 0xE7, 0xD2);

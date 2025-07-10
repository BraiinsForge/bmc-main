// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::Rgb;

pub const LED_COUNT: u8 = 10;
pub const SUB_STEPS: u8 = 31;
pub const SNAKE_LEN: u8 = 4;
pub const MAX_FLIES: u8 = 3;
pub const FIREFLY_STEP_SPEED: f32 = 0.01_f32;
pub const FIREFLY_MIN_BRIGHTNESS: f32 = 0.10_f32; // 10% of max brightness
pub const FIREFLY_SPAWN_PROBABILITY: f64 = 0.005;
pub const FIREFLY_SPAWN_ATTEMPTS: usize = 6;
pub const FIREFLY_SHINE_RADIUS: f32 = 0.20; // 20% of LED strip
pub const DEFAULT_PERIOD: u64 = 2000;

pub const LED_COLOR_RED: Rgb = Rgb::new(0xFF, 0x00, 0x00);
pub const LED_COLOR_MAGENTA: Rgb = Rgb::new(0xFF, 0x00, 0xFF);
pub const LED_COLOR_CYAN: Rgb = Rgb::new(0x00, 0xFF, 0xFF);
pub const LED_COLOR_WARM_WHITE: Rgb = Rgb::new(0xF6, 0xE7, 0xD2);

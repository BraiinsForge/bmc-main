// Copyright (C) 2025  Braiins Systems s.r.o.

use std::time::Duration;

use crate::data::Rgb;

pub const LED_COUNT: u8 = 10;
pub const LED_MAX_BRIGHTNESS: f32 = 1.0;

pub const RGB_VIOLET60: Rgb = Rgb::new(107, 80, 255);
pub const RGB_GREEN: Rgb = Rgb::new(6, 191, 89);
pub const RGB_RED: Rgb = Rgb::new(255, 20, 33);
pub const RGB_ORANGE: Rgb = Rgb::new(255, 122, 13);
pub const RGB_BLACK: Rgb = Rgb::new(0, 0, 0);

pub const ERROR_DURATION: Duration = Duration::from_millis(2000);
pub const SUCCESS_DURATION: Duration = Duration::from_millis(2000);

pub const SNAKE_PERIOD: Duration = Duration::from_millis(1000);
pub const KNIGHT_RIDER_PERIOD: Duration = Duration::from_millis(1000);
pub const BREATHE_PERIOD: Duration = Duration::from_millis(4000);
pub const SOLID_PERIOD: Duration = Duration::from_millis(0);

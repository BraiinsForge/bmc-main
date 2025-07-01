// Copyright (C) 2025  Braiins Systems s.r.o.

pub const LED_COUNT: u8 = 29;
pub const SUB_STEPS: u8 = 31;
pub const SNAKE_LEN: u8 = 10;
pub const MAX_FLIES: u8 = 3;
pub const FLIES_STEP_SPEED: f32 = 0.001_f32;
pub const FLIES_MIN_BRIGHTNESS: f32 = 0.10_f32; // 10% of max brightness
pub const FIREFLY_SHINE_RADIUS: f32 = 0.30; // 30% of LED strip
pub const DEFAULT_PERIOD: u64 = 3000;
pub const APA102_MAX_BRIGHTNESS: u8 = 31; // APA102 max brightness value (5 bits)
pub const SPI_DEV: &str = "/dev/spidev0.1";

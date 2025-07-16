// Copyright (C) 2025  Braiins Systems s.r.o.

pub const APA102_MAX_BRIGHTNESS: u8 = 31; // APA102 max brightness value (5 bits)
pub const RGB_MAX: u8 = 255;
pub const LED_FRACTION_MAX: f32 = 1.0;
pub const LED_MIN_FACTOR: f32 = 0.1;
pub const LED_PHASE_MULTIPLIER: f32 = 2.0;
pub const SUB_STEPS: u8 = 31;
pub const SNAKE_LEN: u8 = 3;
pub const MAX_FLIES: u8 = 2;
pub const FIREFLY_STEP_SPEED: f32 = 0.01_f32;
pub const FIREFLY_MIN_BRIGHTNESS: f32 = 0.10_f32; // 10% of max brightness
pub const FIREFLY_SPAWN_PROBABILITY: f64 = 0.005;
pub const FIREFLY_SPAWN_ATTEMPTS: usize = 6;
pub const FIREFLY_SHINE_RADIUS: f32 = 0.20; // 20% of LED strip
pub const DEFAULT_PERIOD: u64 = 1500;

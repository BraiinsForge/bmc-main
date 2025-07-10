// Copyright (C) 2025  Braiins Systems s.r.o.

// https://github.com/wled/WLED/wiki/List-of-effects-and-palettes

use super::config;
use super::data;
use crate::data::Rgb;
use apa102_spi::{Apa102Pixel, SmartLedsWrite};
use rand::Rng;
use ux::u5;

#[derive(Debug)]
pub struct Firefly {
    pos: u16,
    bright: f32,
    grow: bool,
}

#[derive(Debug, Default)]
pub struct FirefliesState {
    flies: Vec<Firefly>,
}

#[inline]
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_sign_loss)]
fn scale(channel: u8, factor: f32) -> u8 {
    let scaled = f32::from(channel) * factor * factor;
    scaled.clamp(0.0, f32::from(data::RGB_MAX)) as u8
}

/// Snake effect
#[expect(clippy::cast_sign_loss)]
pub fn update_snake<W>(phase: f32, length: u8, brightness: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let real_head_position = phase * f32::from(config::LED_COUNT);
    #[expect(clippy::cast_possible_truncation)]
    let discrete_head_position = real_head_position.floor() as i16;
    let fraction = real_head_position - f32::from(discrete_head_position);

    // Ignore result, since we don't care about the outcome here
    let _ = strip.write((0..config::LED_COUNT).map(|current_led_index| {
        #[expect(clippy::cast_possible_truncation)]
        let current_led_offset = ((u16::from(current_led_index) + u16::from(config::LED_COUNT)
            - discrete_head_position as u16
            + u16::from(config::LED_COUNT))
            % u16::from(config::LED_COUNT)) as u8;

        // If offset is outside of snake's body, do not light the LED
        if current_led_offset >= length {
            Apa102Pixel::default()
        } else {
            let factor = match current_led_offset {
                0 => data::LED_FRACTION_MAX - fraction,
                x if x == length - 1 => fraction * fraction,
                _ => data::LED_FRACTION_MAX,
            };

            Apa102Pixel {
                red: scale(color.r, factor),
                green: scale(color.g, factor),
                blue: scale(color.b, factor),
                brightness: u5::new(brightness),
            }
        }
    }));
}

/// Chase effect
pub fn update_chase<W>(phase: f32, trail_length: u8, brightness: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let time = phase.fract() * data::LED_PHASE_MULTIPLIER * (f32::from(config::LED_COUNT) - 1.0);
    let (real_head_position, _forward) = if time < (f32::from(config::LED_COUNT) - 1.0) {
        (time, true)
    } else {
        (
            data::LED_PHASE_MULTIPLIER * (f32::from(config::LED_COUNT) - 1.0) - time,
            false,
        )
    };

    #[expect(clippy::cast_possible_truncation)]
    let discrete_head_position = real_head_position.floor() as i16;

    // Ignore result, since we don't care about the outcome here
    let _ = strip.write((0..config::LED_COUNT).map(|current_led_index| {
        let distance = (i16::from(current_led_index) - discrete_head_position).abs();
        if distance > i16::from(trail_length) {
            Apa102Pixel::default()
        } else {
            let fade = 1.0 - (f32::from(distance) / (f32::from(trail_length) + 1.0));

            Apa102Pixel {
                red: scale(color.r, fade),
                green: scale(color.g, fade),
                blue: scale(color.b, fade),
                brightness: u5::new(brightness),
            }
        }
    }));
}

/// Scanner effect
pub fn update_scan<W>(phase: f32, length: u8, brightness: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let travel: u16 = u16::from(config::LED_COUNT) + u16::from(length);
    let head_float = phase * f32::from(travel);
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_sign_loss)]
    let head = (head_float as u16) % travel;
    let fraction = head_float - f32::from(head);
    let start = head
        .saturating_sub(u16::from(length) - 1)
        .min(u16::from(config::LED_COUNT) - 1);

    // Ignore result, since we don't care about the outcome here
    let _ = strip.write((0..config::LED_COUNT).map(|index| {
        if u16::from(index) < start || u16::from(index) > head.min(u16::from(config::LED_COUNT) - 1)
        {
            Apa102Pixel::default()
        } else {
            let offset = head - u16::from(index);
            let factor = match offset {
                0 => fraction,
                x if x == u16::from(length) - 1 => 1.0 - fraction,
                _ => 1.0,
            };

            Apa102Pixel {
                red: scale(color.r, factor),
                green: scale(color.g, factor),
                blue: scale(color.b, factor),
                brightness: u5::new(brightness),
            }
        }
    }));
}

// Fireflies effect
#[expect(clippy::cast_sign_loss)]
pub fn update_fireflies<W>(
    state: &mut FirefliesState,
    _phase: f32,
    max_flies: u8,
    brightness: u8,
    color: Rgb,
    strip: &mut W,
) where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let led_count = config::LED_COUNT;
    #[expect(clippy::cast_possible_truncation)]
    let radius = ((f32::from(led_count)) * config::FIREFLY_SHINE_RADIUS).ceil() as i16;
    let spawn_probability = config::FIREFLY_SPAWN_PROBABILITY;
    let mut rng = rand::thread_rng();

    state.flies.retain_mut(|firefly| {
        if firefly.grow {
            firefly.bright += config::FIREFLY_STEP_SPEED;

            if firefly.bright >= data::LED_MAX_BRIGHTNESS {
                firefly.bright = data::LED_MAX_BRIGHTNESS;
                firefly.grow = false;
            }
        } else {
            firefly.bright -= config::FIREFLY_STEP_SPEED;
        }
        firefly.bright > config::FIREFLY_MIN_BRIGHTNESS
    });

    if state.flies.len() < max_flies as usize && rng.gen_bool(spawn_probability) {
        let mut attempts = 0;
        while attempts < config::FIREFLY_SPAWN_ATTEMPTS {
            let position = rng.gen_range(0..u16::from(led_count));
            if state.flies.iter().all(|firefly| firefly.pos != position) {
                state.flies.push(Firefly {
                    pos: position,
                    bright: config::FIREFLY_MIN_BRIGHTNESS,
                    grow: true,
                });
                break;
            }
            attempts += 1;
        }
    }

    let mut frame = vec![0.0_f32; led_count as usize];

    for fly in &state.flies {
        let position = i32::from(fly.pos);
        for distance in -radius..=radius {
            let index = position + i32::from(distance);
            if index < 0 || index >= i32::from(led_count) {
                continue;
            }
            let abs_distance = f32::from(distance.unsigned_abs());
            let weight = 1.0 - abs_distance / f32::from(radius);
            if weight > 0.0 {
                frame[index as usize] += (fly.bright - config::FIREFLY_MIN_BRIGHTNESS) * weight;
            }
        }
    }

    // Convert frame to pixels and write to strip
    let pixels = frame.into_iter().map(|value| {
        let final_brightness =
            (value + config::FIREFLY_MIN_BRIGHTNESS).min(data::LED_MAX_BRIGHTNESS);
        Apa102Pixel {
            red: scale(color.r, final_brightness),
            green: scale(color.g, final_brightness),
            blue: scale(color.b, final_brightness),
            brightness: u5::new(brightness),
        }
    });

    let _ = strip.write(pixels);
}

pub fn update_knight_rider<W>(
    phase: f32,
    core_size: u8,
    fade_size: u8,
    brightness: u8,
    color: Rgb,
    strip: &mut W,
) where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let led_count = config::LED_COUNT;
    let travel = f32::from(led_count - 1);
    let time = phase.fract() * data::LED_PHASE_MULTIPLIER * travel;
    let real_head_position = if time <= travel {
        time
    } else {
        data::LED_PHASE_MULTIPLIER * travel - time
    };

    #[expect(clippy::cast_possible_truncation)]
    let discrete_head_position = real_head_position.floor() as i16;

    #[expect(clippy::integer_division)]
    let half_core_size = (i16::from(core_size)) / 2;
    let min_factor = data::LED_MIN_FACTOR;

    // Ignore result, since we don't care about the outcome here
    let _ = strip.write((0..led_count).map(|current_led_index| {
        let distance = (i16::from(current_led_index) - discrete_head_position).abs();
        let factor = if distance > half_core_size + i16::from(fade_size) {
            min_factor
        } else if distance <= half_core_size {
            1.0
        } else {
            let fade_factor =
                1.0 - f32::from(distance - half_core_size) / (f32::from(fade_size.max(1)));
            fade_factor.max(min_factor)
        };

        Apa102Pixel {
            red: scale(color.r, factor),
            green: scale(color.g, factor),
            blue: scale(color.b, factor),
            brightness: u5::new(brightness),
        }
    }));
}

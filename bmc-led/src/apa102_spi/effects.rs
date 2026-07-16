// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

// https://github.com/wled/WLED/wiki/List-of-effects-and-palettes

use super::config::{LED_FRACTION_MAX, LED_MIN_FACTOR, LED_PHASE_MULTIPLIER, RGB_MAX};

use crate::data::Rgb;
use apa102_spi::{Apa102Pixel, SmartLedsWrite};
use ux::u5;

#[inline]
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_sign_loss)]
fn scale(channel: u8, factor: f32) -> u8 {
    let scaled = f32::from(channel) * factor * factor;
    scaled.clamp(0.0, f32::from(RGB_MAX)) as u8
}

/// Snake effect
#[expect(clippy::cast_sign_loss)]
pub fn update_snake<W>(phase: f32, length: u8, brightness: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let real_head_position = phase * f32::from(crate::config::LED_COUNT);
    #[expect(clippy::cast_possible_truncation)]
    let discrete_head_position = real_head_position.floor() as i16;
    let fraction = real_head_position - f32::from(discrete_head_position);

    // Ignore result, since we don't care about the outcome here
    let _ = strip.write((0..crate::config::LED_COUNT).map(|current_led_index| {
        #[expect(clippy::cast_possible_truncation)]
        let current_led_offset = ((u16::from(current_led_index)
            + u16::from(crate::config::LED_COUNT)
            - discrete_head_position as u16
            + u16::from(crate::config::LED_COUNT))
            % u16::from(crate::config::LED_COUNT)) as u8;

        // If offset is outside of snake's body, do not light the LED
        if current_led_offset >= length {
            Apa102Pixel::default()
        } else {
            let factor = match current_led_offset {
                0 => LED_FRACTION_MAX - fraction,
                x if x == length - 1 => fraction * fraction,
                _ => LED_FRACTION_MAX,
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
    let time = phase.fract() * LED_PHASE_MULTIPLIER * (f32::from(crate::config::LED_COUNT) - 1.0);
    let (real_head_position, _forward) = if time < (f32::from(crate::config::LED_COUNT) - 1.0) {
        (time, true)
    } else {
        (
            LED_PHASE_MULTIPLIER * (f32::from(crate::config::LED_COUNT) - 1.0) - time,
            false,
        )
    };

    #[expect(clippy::cast_possible_truncation)]
    let discrete_head_position = real_head_position.floor() as i16;

    // Ignore result, since we don't care about the outcome here
    let _ = strip.write((0..crate::config::LED_COUNT).map(|current_led_index| {
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
    let travel: u16 = u16::from(crate::config::LED_COUNT) + u16::from(length);
    let head_float = phase * f32::from(travel);
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_sign_loss)]
    let head = (head_float as u16) % travel;
    let fraction = head_float - f32::from(head);
    let start = head
        .saturating_sub(u16::from(length) - 1)
        .min(u16::from(crate::config::LED_COUNT) - 1);

    // Ignore result, since we don't care about the outcome here
    let _ = strip.write((0..crate::config::LED_COUNT).map(|index| {
        if u16::from(index) < start
            || u16::from(index) > head.min(u16::from(crate::config::LED_COUNT) - 1)
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
    let led_count = crate::config::LED_COUNT;
    let travel = f32::from(led_count - 1);
    let time = phase.fract() * LED_PHASE_MULTIPLIER * travel;
    let real_head_position = if time <= travel {
        time
    } else {
        LED_PHASE_MULTIPLIER * travel - time
    };

    #[expect(clippy::cast_possible_truncation)]
    let discrete_head_position = real_head_position.floor() as i16;

    #[expect(clippy::integer_division)]
    let half_core_size = (i16::from(core_size)) / 2;
    let min_factor = LED_MIN_FACTOR;

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

pub fn update_none<W>(strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let _ = strip.write((0..crate::config::LED_COUNT).map(|_| Apa102Pixel::default()));
}

pub fn update_solid<W>(brightness: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let _ = strip.write((0..crate::config::LED_COUNT).map(|_| Apa102Pixel {
        red: color.r,
        green: color.g,
        blue: color.b,
        brightness: u5::new(brightness),
    }));
}

pub fn update_breathe<W>(phase: f32, brightness: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let factor = (phase * std::f32::consts::PI * 2.0).sin() * 0.5 + 0.5;
    let _ = strip.write((0..crate::config::LED_COUNT).map(|_| Apa102Pixel {
        red: scale(color.r, factor),
        green: scale(color.g, factor),
        blue: scale(color.b, factor),
        brightness: u5::new(brightness),
    }));
}

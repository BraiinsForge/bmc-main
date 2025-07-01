// Copyright (C) 2025  Braiins Systems s.r.o.

// https://github.com/wled/WLED/wiki/List-of-effects-and-palettes

use apa102_spi::{Apa102Pixel, SmartLedsWrite};

use super::config;
use crate::data::Rgb;
use rand::Rng;
use ux::u5;

#[derive(Debug)]
pub struct Firefly {
    pos: u16,
    bright: f32,
    grow: bool,
}

#[derive(Debug)]
pub struct FirefliesState {
    flies: Vec<Firefly>,
}

#[inline]
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_sign_loss)]
fn scale(channel: u8, factor: f32) -> u8 {
    (f32::from(channel) * factor * factor) as u8 // simple gamma ≈2.0
}

/// Snake effect
#[expect(clippy::cast_sign_loss)]
pub fn update_snake<W>(phase: f32, len: u8, br: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let head_pos = phase * f32::from(config::LED_COUNT);

    #[expect(clippy::cast_possible_truncation)]
    let head_idx = head_pos as u8;
    let frac = head_pos - f32::from(head_idx);

    strip
        .write((0..config::LED_COUNT).map(|i| {
            let off = (i + config::LED_COUNT - head_idx) % config::LED_COUNT;
            if off >= len {
                return Apa102Pixel {
                    red: 0,
                    green: 0,
                    blue: 0,
                    brightness: u5::new(0),
                };
            }
            let f = match off {
                0 => 1.0 - frac,
                x if x == len - 1 => frac * frac,
                _ => 1.0,
            };

            Apa102Pixel {
                red: scale(color.r, f),
                green: scale(color.g, f),
                blue: scale(color.b, f),
                brightness: u5::new(br),
            }
        }))
        .ok();
}

/// Chase effect
pub fn update_chase<W>(phase: f32, trail: u8, br: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let n = config::LED_COUNT;
    let t = phase.fract() * 2.0 * (f32::from(n) - 1.0);
    let (head_pos, _forward) = if t < (f32::from(n) - 1.0) {
        (t, true)
    } else {
        (2.0 * (f32::from(n) - 1.0) - t, false)
    };

    #[expect(clippy::cast_possible_truncation)]
    let head_idx = head_pos.floor() as i16;

    strip
        .write((0..n).map(|i| {
            let dist = (i16::from(i) - head_idx).abs();
            if dist > i16::from(trail) {
                Apa102Pixel {
                    red: 0,
                    green: 0,
                    blue: 0,
                    brightness: u5::new(0),
                }
            } else {
                let fade = 1.0 - (f32::from(dist) / (f32::from(trail) + 1.0));
                Apa102Pixel {
                    red: scale(color.r, fade),
                    green: scale(color.g, fade),
                    blue: scale(color.b, fade),
                    brightness: u5::new(br),
                }
            }
        }))
        .ok();
}

/// Scanner effect
pub fn update_scan<W>(phase: f32, len: u8, br: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let travel: u16 = u16::from(config::LED_COUNT) + u16::from(len);
    let head_f = phase * f32::from(travel);
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_sign_loss)]
    let head = (head_f as u16) % travel;
    let frac = head_f - f32::from(head);
    let start = head
        .saturating_sub(u16::from(len) - 1)
        .min(u16::from(config::LED_COUNT) - 1);

    strip
        .write((0..config::LED_COUNT).map(|idx| {
            if u16::from(idx) < start || u16::from(idx) > head.min(u16::from(config::LED_COUNT) - 1)
            {
                return Apa102Pixel {
                    red: 0,
                    green: 0,
                    blue: 0,
                    brightness: u5::new(0),
                };
            }
            let off = head - u16::from(idx);
            let f = match off {
                0 => frac,
                x if x == u16::from(len) - 1 => 1.0 - frac,
                _ => 1.0,
            };
            Apa102Pixel {
                red: scale(color.r, f),
                green: scale(color.g, f),
                blue: scale(color.b, f),
                brightness: u5::new(br),
            }
        }))
        .ok();
}

impl FirefliesState {
    #[must_use]
    pub const fn default() -> Self {
        FirefliesState { flies: Vec::new() }
    }
}

// Fireflies effect
#[expect(clippy::cast_sign_loss)]
pub fn update_fireflies<W>(
    state: &mut FirefliesState,
    _phase: f32,
    max_flies: u8,
    br: u8,
    color: Rgb,
    strip: &mut W,
) where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let n = config::LED_COUNT;
    #[expect(clippy::cast_possible_truncation)]
    let radius = ((f32::from(n)) * config::FIREFLY_SHINE_RADIUS).ceil() as i16;
    let spawn_p = 0.005;
    let mut rng = rand::thread_rng();

    state.flies.retain_mut(|f| {
        if f.grow {
            f.bright += config::FLIES_STEP_SPEED;

            if f.bright >= 1.0 {
                f.bright = 1.0;
                f.grow = false;
            }
        } else {
            f.bright -= config::FLIES_STEP_SPEED;
        }
        f.bright > config::FLIES_MIN_BRIGHTNESS
    });

    if state.flies.len() < max_flies as usize && rng.gen_bool(spawn_p) {
        let mut attempts = 0;
        while attempts < 6 {
            let pos = rng.gen_range(0..u16::from(n));
            if state.flies.iter().all(|f| f.pos != pos) {
                state.flies.push(Firefly {
                    pos,
                    bright: config::FLIES_MIN_BRIGHTNESS,
                    grow: true,
                });
                break;
            }
            attempts += 1;
        }
    }

    let mut frame = vec![0.0_f32; n as usize];

    for fly in &state.flies {
        let p = i32::from(fly.pos);
        for d in -radius..=radius {
            let idx = p + i32::from(d);
            if idx < 0 || idx >= i32::from(n) {
                continue;
            }
            let dist = f32::from(d.unsigned_abs());
            let w = 1.0 - dist / f32::from(radius);
            if w > 0.0 {
                frame[idx as usize] += (fly.bright - config::FLIES_MIN_BRIGHTNESS) * w;
            }
        }
    }

    for val in &mut frame {
        *val = (*val + config::FLIES_MIN_BRIGHTNESS).min(1.0);
    }

    let _ = strip.write(frame.into_iter().map(|f| Apa102Pixel {
        red: scale(color.r, f),
        green: scale(color.g, f),
        blue: scale(color.b, f),
        brightness: u5::new(br),
    }));
}

pub fn update_knight_rider<W>(phase: f32, core: u8, fade: u8, br: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let n = config::LED_COUNT;
    let travel = f32::from(n - 1);
    let t = phase.fract() * 2.0 * travel;
    let head_pos = if t <= travel { t } else { 2.0 * travel - t };

    #[expect(clippy::cast_possible_truncation)]
    let head_idx = head_pos.floor() as i16;

    #[expect(clippy::integer_division)]
    let half_core = (i16::from(core)) / 2;
    let min_factor = 0.1; // TODO: Configurable

    strip
        .write((0..n).map(|i| {
            let dist = (i16::from(i) - head_idx).abs();
            let factor = if dist > half_core + i16::from(fade) {
                min_factor
            } else if dist <= half_core {
                1.0
            } else {
                let fade_factor = 1.0 - f32::from(dist - half_core) / (f32::from(fade.max(1)));
                fade_factor.max(min_factor)
            };
            Apa102Pixel {
                red: scale(color.r, factor),
                green: scale(color.g, factor),
                blue: scale(color.b, factor),
                brightness: u5::new(br),
            }
        }))
        .ok();
}

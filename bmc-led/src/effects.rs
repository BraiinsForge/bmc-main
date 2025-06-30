// https://github.com/wled/WLED/wiki/List-of-effects-and-palettes

use crate::data::Rgb;

use apa102_spi::{Apa102Pixel, SmartLedsWrite};

use super::config;

use rand::Rng;
use ux::u5;

#[derive(Debug)]
pub struct Firefly {
    pos: usize,
    bright: f32,
    grow: bool, // true → zesiluje, false → pohasíná
}

#[derive(Debug)]
pub struct FirefliesState {
    flies: Vec<Firefly>,
}

#[inline]
fn scale(channel: u8, factor: f32) -> u8 {
    (channel as f32 * factor * factor) as u8 // simple gamma ≈2.0
}

/// Snake effect
pub fn update_snake<W>(phase: f32, len: usize, br: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let head_pos = phase * config::LED_COUNT as f32;
    let head_idx = head_pos as usize;
    let frac = head_pos - head_idx as f32;

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
pub fn update_chase<W>(phase: f32, trail: usize, br: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let n = config::LED_COUNT as isize;
    let t = phase.fract() * 2.0 * (n as f32 - 1.0);
    let (head_pos, _forward) = if t < (n as f32 - 1.0) {
        (t, true)
    } else {
        (2.0 * (n as f32 - 1.0) - t, false)
    };
    let head_idx = head_pos.floor() as isize;

    strip
        .write((0..n).map(|i| {
            let dist = (i - head_idx).abs();
            if dist > trail as isize {
                Apa102Pixel {
                    red: 0,
                    green: 0,
                    blue: 0,
                    brightness: u5::new(0),
                }
            } else {
                let fade = 1.0 - (dist as f32 / (trail as f32 + 1.0));
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
pub fn update_scan<W>(phase: f32, len: usize, br: u8, color: Rgb, strip: &mut W)
where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let travel = config::LED_COUNT + len;
    let head_f = phase * travel as f32;
    let head = (head_f as usize) % travel;
    let frac = head_f - head as f32;
    let start = head.saturating_sub(len - 1).min(config::LED_COUNT - 1);

    strip
        .write((0..config::LED_COUNT).map(|idx| {
            if idx < start || idx > head.min(config::LED_COUNT - 1) {
                return Apa102Pixel {
                    red: 0,
                    green: 0,
                    blue: 0,
                    brightness: u5::new(0),
                };
            }
            let off = head - idx;
            let f = match off {
                0 => frac,
                x if x == len - 1 => 1.0 - frac,
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
    pub const fn default() -> Self {
        FirefliesState { flies: Vec::new() }
    }
}

// Fireflies effect
pub fn update_fireflies<W>(
    state: &mut FirefliesState,
    _phase: f32,
    max_flies: usize,
    br: u8,
    color: Rgb,
    strip: &mut W,
) where
    W: SmartLedsWrite<Color = Apa102Pixel>,
{
    let n = config::LED_COUNT as isize;
    let radius = ((n as f32) * config::FIREFLY_SHINE_RADIUS).ceil() as isize;
    let spawn_p = 0.02;
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

    if state.flies.len() < max_flies && rng.gen_bool(spawn_p) {
        let mut attempts = 0;
        while attempts < 6 {
            let pos = rng.gen_range(0..n as usize);
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

    let mut frame = vec![0.0f32; n as usize];

    for fly in &state.flies {
        let p = fly.pos as isize;
        for d in -radius..=radius {
            let idx = p + d;
            if idx < 0 || idx >= n {
                continue;
            }
            let dist = d.unsigned_abs() as f32;
            let w = 1.0 - dist / radius as f32;
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

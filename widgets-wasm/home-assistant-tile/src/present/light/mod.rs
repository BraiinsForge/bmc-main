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

//! Lights: the card turns one on and off; one that dims shows its brightness,
//! and where it takes them, its colour temperature and hue, as far as the card has room.

use bmc_wasm_sdk::*;
use home_assistant::Entity;
use home_assistant::domain::light::{self, HueSaturation};
use home_assistant::domain::on_off;
use home_assistant::format::with_unit;

use super::{CardAction, Context, Presenter, card, card_action};
use crate::interaction::pending::{Pending, Request, Slot, Timing};
use crate::view::blocks::badge::{Disc, Mark};
use crate::view::blocks::chips::{Chip, Chips, Role};
use crate::view::blocks::gradient;
use crate::view::blocks::stepper::{Ends, Gradient, Level, Stepper, Touched, Track};
use crate::view::card::{self as card_view, Card, Line, StateDisplay};
use crate::view::style::{Accent, Icon};
use crate::view::touches::Touches;

pub const POWER: Slot = Slot::new("power");
pub const BRIGHTNESS: Slot = Slot::new("brightness");
pub const WARMTH: Slot = Slot::new("warmth");
pub const HUE: Slot = Slot::new("hue");
const PRESETS: &str = "presets";

/// − and + move to the next multiple of this, and − from it or below turns the light off.
const STEP: u8 = 10;
const PERCENTS: [u8; 4] = [25, 50, 75, 100];
/// A step of colour temperature, as a share of the light's range.
const WARMTH_STEP: f32 = 0.1;
const HUE_STEP: f32 = 30.0;
const FULL_CIRCLE: f32 = 360.0;
/// The saturation a hue picked on the tile is sent at: the tile picks no saturation.
const SATURATION: f64 = 100.0;
/// A brightness a light that is on does not report.
const UNKNOWN: &str = "\u{2013}";
const UNKNOWN_POSITION: &str = "Unknown";
const ON: &str = "On";
const OFF: &str = "Off";
/// The frontend's `state-badge`: an icon colour paler than this is a white, drawn at [`WHITE_VALUE`] of 255…
const WHITE_SATURATION: f32 = 0.1;
const WHITE_VALUE: f32 = 225.0;
/// … and one paler than this is saturated up to it.
const PALE_SATURATION: f32 = 0.4;

const LIGHTBULB: Svg = include_svg!("assets/mdi/lightbulb.svg");
const LIGHTBULB_OFF: Svg = include_svg!("assets/mdi/lightbulb-off.svg");
/// The frontend's icon for a light at rest.
pub(super) fn icon() -> Icon {
    Icon(&LIGHTBULB)
}

/// The colour temperatures a light takes, in kelvin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kelvins {
    pub warmest: f32,
    pub coolest: f32,
}

impl From<light::KelvinRange> for Kelvins {
    fn from(range: light::KelvinRange) -> Self {
        Self {
            warmest: narrow(range.warmest),
            coolest: narrow(range.coolest),
        }
    }
}

impl Default for Kelvins {
    fn default() -> Self {
        light::DEFAULT_KELVIN_RANGE.into()
    }
}

impl Kelvins {
    fn share_of(self, kelvin: f32) -> f32 {
        let span = self.coolest - self.warmest;
        if span > 0.0 {
            ((kelvin - self.warmest) / span).clamp(0.0, 1.0)
        } else {
            0.5
        }
    }

    fn at(self, share: f32) -> f32 {
        (self.warmest + share.clamp(0.0, 1.0) * (self.coolest - self.warmest)).round()
    }
}

/// What the tile reads of a light.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// None while Home Assistant knows no position for it.
    pub on: Option<bool>,
    /// In percent, from 1 to 100; Home Assistant reports none while the light is off.
    pub brightness: Option<u8>,
    pub dims: bool,
    /// Its range, where it takes a colour temperature.
    pub warmth: Option<Kelvins>,
    pub kelvin: Option<f32>,
    pub colours: bool,
    pub colour: Option<HueSaturation>,
}

impl State {
    #[must_use]
    pub fn read(entity: &Entity) -> Self {
        Self {
            on: on_off(entity),
            brightness: light::brightness_percent(entity),
            dims: light::dims(entity),
            warmth: light::kelvin_range(entity).map(Kelvins::from),
            kelvin: light::kelvin(entity).map(narrow),
            colours: light::takes_colour(entity),
            colour: light::colour(entity),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a colour temperature, a hue or a saturation, a few thousand at most"
)]
fn narrow(number: f64) -> f32 {
    number as f32
}

/// What is shown: what the user asked for while Home Assistant has yet to confirm it,
/// else what Home Assistant reports.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Shown<T> {
    value: T,
    /// Drawn faint: a value from a run of − and + taps until Home Assistant confirms it.
    /// One the finger picked outright is not, so a tap on the track or a preset does not flicker.
    faint: bool,
}

impl<T> Shown<T> {
    fn over(reported: T, wanted: Option<T>, by_taps: bool) -> Self {
        match wanted {
            Some(value) => Self {
                value,
                faint: by_taps,
            },
            None => Self {
                value: reported,
                faint: false,
            },
        }
    }
}

/// A row of blocks under the heading, in the order a card with less room drops them from the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Brightness,
    Warmth,
    Hue,
    Presets,
}

/// One frame of a light's tile.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    card: Card,
    card_action: CardAction,
    rows: Vec<Row>,
    width: f32,
    can_act: bool,
    on: bool,
    /// Only for a light that dims.
    brightness: Option<Shown<Option<u8>>>,
    warmth: Option<(Kelvins, Shown<Option<f32>>)>,
    hue: Option<Shown<Option<f32>>>,
}

impl Tile {
    /// `state` with what the user asked for laid over it.
    #[must_use]
    pub fn new(state: &State, context: &Context) -> Self {
        let pending = context.pending;
        let wanted_brightness = pending.number(BRIGHTNESS).map(whole_percent);
        let turning_on = [BRIGHTNESS, WARMTH, HUE]
            .into_iter()
            .any(|slot| pending.asked(slot));
        let position = pending
            .on(POWER)
            .or(turning_on.then_some(true))
            .or(state.on);
        let on = position == Some(true);
        let brightness = state.dims.then(|| {
            Shown::over(
                state.brightness,
                wanted_brightness.map(Some),
                pending.asked_by_taps(BRIGHTNESS),
            )
        });
        let percent = brightness.and_then(|shown| shown.value).filter(|_| on);

        let room = context.room;
        let fits = card_view::rows((room.width, room.height), room.size);
        let candidates = [
            (Row::Brightness, state.dims),
            (Row::Warmth, state.warmth.is_some()),
            (Row::Hue, state.colours),
            (Row::Presets, state.dims),
        ];
        let rows: Vec<Row> = candidates
            .into_iter()
            .filter_map(|(row, shown)| shown.then_some(row))
            .take(fits.count)
            .collect();

        let color = light_color(state, pending);
        let request = turn(!on);
        // Unlike a button's, a light's tile set to neither tap nor hold is a status on purpose, not a mistake.
        let card_action = if context.gesture.can_fire() {
            card_action(context, true, request)
        } else {
            CardAction::None
        };
        let display = StateDisplay {
            line: Some(Line::State(line(
                position,
                percent.filter(|_| !rows.contains(&Row::Brightness)),
            ))),
            color: on
                .then(|| color.map_or(Accent::Amber, |color| Accent::Light(icon_color(color)))),
            icon: Icon(if on { &LIGHTBULB } else { &LIGHTBULB_OFF }),
            disc: if on { Disc::Tinted } else { Disc::Bare },
            mark: position.is_none().then_some(Mark::Unknown),
        };
        Self {
            card: card(context, display, &card_action),
            card_action,
            rows,
            width: fits.width,
            can_act: context.can_act(),
            on,
            brightness,
            warmth: state.warmth.map(|range| {
                (
                    range,
                    Shown::over(
                        state.kelvin,
                        pending.number(WARMTH).map(Some),
                        pending.asked_by_taps(WARMTH),
                    ),
                )
            }),
            hue: state.colours.then(|| {
                Shown::over(
                    state.colour.map(|colour| narrow(colour.hue)),
                    pending.number(HUE).map(Some),
                    pending.asked_by_taps(HUE),
                )
            }),
        }
    }

    /// The brightness while on, as far as it is known.
    fn percent(&self) -> Option<u8> {
        self.brightness?.value.filter(|_| self.on)
    }

    fn shows(&self, row: Row) -> bool {
        self.rows.contains(&row)
    }

    fn brightness(&self) -> Option<Stepper> {
        let brightness = self.brightness.filter(|_| self.shows(Row::Brightness))?;
        let percent = self.percent();
        Some(Stepper {
            key: BRIGHTNESS.name().to_owned(),
            ends: Ends::Signs,
            down: self.can_act && percent.is_some(),
            up: self.can_act && (!self.on || percent.is_some_and(|percent| percent < 100)),
            pending: brightness.faint && percent.is_some(),
            track: Track::Level {
                value: match (self.on, percent) {
                    (false, _) => OFF.to_owned(),
                    (true, Some(percent)) => percent_text(percent),
                    (true, None) => UNKNOWN.to_owned(),
                },
                level: Level {
                    share: percent.map_or(0.0, |percent| f32::from(percent) / 100.0),
                    color: Accent::Amber.color(),
                },
            },
            width: self.width,
        })
    }

    fn presets(&self) -> Option<Chips> {
        if !self.shows(Row::Presets) {
            return None;
        }
        let percent = self.percent();
        Some(Chips {
            key: PRESETS.to_owned(),
            chips: PERCENTS
                .map(|preset| Chip {
                    label: percent_text(preset),
                    icon: None,
                    accent: Some(Accent::Amber.color()),
                    enabled: self.can_act,
                })
                .to_vec(),
            selected: PERCENTS.iter().position(|preset| Some(*preset) == percent),
            role: Role::Options,
            width: self.width,
        })
    }

    fn warmth(&self) -> Option<Stepper> {
        let (range, kelvin) = self.warmth.filter(|_| self.shows(Row::Warmth))?;
        Some(Stepper {
            key: WARMTH.name().to_owned(),
            ends: Ends::Signs,
            down: self.can_act && kelvin.value.is_none_or(|kelvin| kelvin > range.warmest),
            up: self.can_act && kelvin.value.is_none_or(|kelvin| kelvin < range.coolest),
            pending: kelvin.faint,
            track: Track::Gradient(Gradient {
                stops: gradient::temperatures(range.warmest, range.coolest),
                share: kelvin.value.map(|kelvin| range.share_of(kelvin)),
            }),
            width: self.width,
        })
    }

    fn hue(&self) -> Option<Stepper> {
        let hue = self.hue.filter(|_| self.shows(Row::Hue))?;
        Some(Stepper {
            key: HUE.name().to_owned(),
            ends: Ends::Signs,
            down: self.can_act && hue.value.is_none_or(|hue| hue > 0.0),
            up: self.can_act && hue.value.is_none_or(|hue| hue < FULL_CIRCLE),
            pending: hue.faint,
            track: Track::Gradient(Gradient {
                stops: gradient::hues().to_vec(),
                share: hue.value.map(|hue| hue / FULL_CIRCLE),
            }),
            width: self.width,
        })
    }

    /// A dimmer's rules: + turns a light that is off on at the brightness it had,
    /// − from 10 % or below turns it off, and a tap on the bar never does.
    fn brightness_after(&self, touched: Touched) -> Option<Request> {
        match (touched, self.on, self.percent()) {
            (Touched::Set(share), _, _) => {
                Some(set_brightness(percent_of(share).max(1), Timing::Now))
            }
            (Touched::Up, false, _) => Some(turn(true)),
            (Touched::Up, true, Some(percent)) => Some(set_brightness(
                ((percent / STEP + 1) * STEP).min(100),
                Timing::AfterTaps,
            )),
            (Touched::Down, true, Some(percent)) => {
                let next = percent.saturating_sub(1) / STEP * STEP;
                Some(if next >= STEP {
                    set_brightness(next, Timing::AfterTaps)
                } else {
                    turn(false)
                })
            }
            (Touched::Up | Touched::Down, true, None) | (Touched::Down, false, _) => None,
        }
    }

    /// A light that shows no colour temperature steps from the middle of its range.
    /// It drops a hue still asked for, as a light shows one or the other.
    fn warmth_after(&self, touched: Touched) -> Option<Request> {
        let (range, kelvin) = self.warmth?;
        let current = kelvin.value.map_or(0.5, |kelvin| range.share_of(kelvin));
        let (share, timing) = match touched {
            Touched::Set(share) => (share, Timing::Now),
            Touched::Up => (current + WARMTH_STEP, Timing::AfterTaps),
            Touched::Down => (current - WARMTH_STEP, Timing::AfterTaps),
        };
        let kelvin = range.at(share);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a colour temperature of a few thousand kelvin, rounded"
        )]
        let whole_kelvin = kelvin as u32;
        Some(
            Request::number(
                WARMTH,
                kelvin,
                light::turn_on_at_kelvin(whole_kelvin),
                timing,
            )
            .superseding(&[HUE, POWER]),
        )
    }

    /// A light that shows no hue steps from the middle of the circle.
    /// It drops a colour temperature still asked for, as a light shows one or the other.
    fn hue_after(&self, touched: Touched) -> Option<Request> {
        let current = self.hue?.value.unwrap_or(FULL_CIRCLE / 2.0);
        let (hue, timing) = match touched {
            Touched::Set(share) => (share * FULL_CIRCLE, Timing::Now),
            Touched::Up => (current + HUE_STEP, Timing::AfterTaps),
            Touched::Down => (current - HUE_STEP, Timing::AfterTaps),
        };
        let hue = hue.clamp(0.0, FULL_CIRCLE).round();
        Some(
            Request::number(
                HUE,
                hue,
                light::turn_on_at_hue(f64::from(hue), SATURATION),
                timing,
            )
            .superseding(&[WARMTH, POWER]),
        )
    }
}

impl Presenter for Tile {
    fn view(&self, touches: &Touches) -> Node {
        let rows = self
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Brightness => self.brightness().map(|stepper| stepper.view(touches)),
                Row::Warmth => self.warmth().map(|stepper| stepper.view(touches)),
                Row::Hue => self.hue().map(|stepper| stepper.view(touches)),
                Row::Presets => self.presets().map(|chips| chips.view(touches)),
            })
            .collect();
        self.card.with_rows(rows, touches)
    }

    fn touched(&self, touches: &Touches) -> Option<Request> {
        if let Some(touch) = self
            .brightness()
            .and_then(|stepper| stepper.touched(touches))
        {
            return self.brightness_after(touch);
        }
        if let Some(chip) = self.presets().and_then(|chips| chips.touched(touches)) {
            return Some(set_brightness(PERCENTS[chip], Timing::Now));
        }
        if let Some(touch) = self.warmth().and_then(|stepper| stepper.touched(touches)) {
            return self.warmth_after(touch);
        }
        let touch = self.hue()?.touched(touches)?;
        self.hue_after(touch)
    }

    fn card_action(&self) -> CardAction {
        self.card_action.clone()
    }
}

/// The light's colour at full brightness, what the user asked for over what Home Assistant reports;
/// none for a light that tells neither a colour temperature nor a colour.
fn light_color(state: &State, pending: &Pending) -> Option<Color> {
    match (pending.number(HUE), pending.number(WARMTH)) {
        (Some(hue), _) => Some(hue_color(hue, narrow(SATURATION))),
        (None, Some(kelvin)) => Some(gradient::temperature_color(kelvin)),
        (None, None) => match (state.kelvin, state.colour) {
            (Some(kelvin), _) => Some(gradient::temperature_color(kelvin)),
            (None, Some(colour)) => Some(hue_color(narrow(colour.hue), narrow(colour.saturation))),
            (None, None) => None,
        },
    }
}

/// The icon's colour for a light's own: the frontend's `state-badge` saturates a pale colour
/// and greys a white one, so the icon still reads as lit beside the neutral icon of a light that is off.
fn icon_color(color: Color) -> Color {
    let channels = [color.red(), color.green(), color.blue()].map(f32::from);
    let max = channels.iter().copied().fold(0.0, f32::max);
    let min = channels.iter().copied().fold(max, f32::min);
    if max <= 0.0 {
        return color;
    }
    let saturation = (max - min) / max;
    if saturation < WHITE_SATURATION {
        color.brightness(WHITE_VALUE / max)
    } else if saturation < PALE_SATURATION {
        let stretch = PALE_SATURATION / saturation;
        let [red, green, blue] =
            channels.map(|channel| gradient::channel(max - (max - channel) * stretch));
        Color::from_rgb(red, green, blue)
    } else {
        color
    }
}

/// A colour at full value, its `saturation` in percent.
fn hue_color(hue: f32, saturation: f32) -> Color {
    Color::from_hsv(hue, saturation / 100.0, 1.0)
}

/// The line under the name: as the frontend's tile tells a light, its brightness while on,
/// unless its bar already shows it.
fn line(position: Option<bool>, percent: Option<u8>) -> String {
    match (position, percent) {
        (None, _) => UNKNOWN_POSITION.to_owned(),
        (Some(false), _) => OFF.to_owned(),
        (Some(true), Some(percent)) => percent_text(percent),
        (Some(true), None) => ON.to_owned(),
    }
}

/// The light turned on at `percent`.
fn set_brightness(percent: u8, timing: Timing) -> Request {
    Request::number(
        BRIGHTNESS,
        f32::from(percent),
        light::turn_on_at_percent(percent),
        timing,
    )
    .superseding(&[POWER])
}

/// Off drops what still waits to be sent, which would turn it back on.
fn turn(on: bool) -> Request {
    if on {
        Request::flip(POWER, true, light::turn_on())
    } else {
        Request::flip(POWER, false, light::turn_off()).superseding(&[BRIGHTNESS, WARMTH, HUE])
    }
}

/// `share` of the way from 0 to 100 %, in whole percent.
fn percent_of(share: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=100 first"
    )]
    let percent = (share * 100.0).round().clamp(0.0, 100.0) as u8;
    percent
}

fn whole_percent(number: f32) -> u8 {
    percent_of(number / 100.0)
}

fn percent_text(percent: u8) -> String {
    with_unit(fmt!("{percent}"), Some("%"))
}

#[cfg(test)]
mod tests;

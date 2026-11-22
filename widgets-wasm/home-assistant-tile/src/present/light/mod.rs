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
use home_assistant::domain::light::{
    self, Brightness, ColorTemperature, Dimming, KelvinRange, Look,
};
use home_assistant::domain::on_off;
use home_assistant::format::with_unit;

use super::{CardAction, Context, Position, Presenter, card, card_action};
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
const PERCENTS: [Brightness; 4] = [
    Brightness::from_percent(25),
    Brightness::from_percent(50),
    Brightness::from_percent(75),
    Brightness::FULL,
];
/// A step of colour temperature, as a share of the light's range.
const WARMTH_STEP: f32 = 0.1;
const HUE_STEP: f32 = 30.0;
const FULL_CIRCLE: f32 = 360.0;
/// The saturation a hue picked on the tile is sent at: the tile picks no saturation.
const SATURATION: f64 = 100.0;
/// A brightness a light that is on does not report.
const UNKNOWN: &str = "\u{2013}";
/// The strip's colour for a light that tells neither a colour temperature nor a colour: a warm white bulb.
const PLAIN_KELVIN: ColorTemperature = ColorTemperature::from_kelvin(2700.0);
/// The dimmest share of full the strip previews a light at; dimmer reads as off.
const LED_FLOOR: f32 = 0.1;
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

/// What the tile reads of a light.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub position: Position,
    /// Home Assistant reports none while the light is off.
    pub brightness: Option<Brightness>,
    /// None for a light that only switches.
    pub dimming: Option<Dimming>,
    pub look: Look,
}

impl State {
    #[must_use]
    pub fn read(entity: &Entity) -> Self {
        Self {
            position: Position::of(on_off(entity)),
            brightness: light::brightness(entity),
            dimming: light::dimming(entity),
            look: light::look(entity),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a hue or a saturation, 360 at most"
)]
fn narrow(number: f64) -> f32 {
    number as f32
}

/// What is shown: what the user asked for while Home Assistant has yet to confirm it,
/// else what Home Assistant reports.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Shown<T> {
    /// None while Home Assistant reports none, such as for a light that is off.
    value: Option<T>,
    /// Drawn faint: a value from a run of − and + taps until Home Assistant confirms it.
    /// One the finger picked outright is not, so a tap on the track or a preset does not flicker.
    faint: bool,
}

impl<T> Shown<T> {
    fn over(reported: Option<T>, wanted: Option<T>, by_taps: bool) -> Self {
        match wanted {
            Some(value) => Self {
                value: Some(value),
                faint: by_taps,
            },
            None => Self {
                value: reported,
                faint: false,
            },
        }
    }
}

/// The values on the rows of a light that dims.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Dimmer {
    brightness: Shown<Brightness>,
    /// Where it takes a colour temperature.
    warmth: Option<Warmth>,
    /// In degrees, where it takes a colour.
    hue: Option<Shown<f32>>,
}

impl Dimmer {
    fn new(state: &State, dimming: Dimming, pending: &Pending) -> Self {
        let (kelvin, hue) = match state.look {
            Look::White(kelvin) => (Some(kelvin), None),
            Look::Colour(colour) => (None, Some(narrow(colour.hue))),
            Look::Plain | Look::Unknown => (None, None),
        };
        Self {
            brightness: Shown::over(
                state.brightness,
                pending.number(BRIGHTNESS).map(brightness_of),
                pending.asked_by_taps(BRIGHTNESS),
            ),
            warmth: dimming.warmth.map(|range| Warmth {
                range,
                shown: Shown::over(
                    kelvin,
                    pending.number(WARMTH).map(ColorTemperature::from_kelvin),
                    pending.asked_by_taps(WARMTH),
                ),
            }),
            hue: dimming
                .colour
                .then(|| Shown::over(hue, pending.number(HUE), pending.asked_by_taps(HUE))),
        }
    }
}

/// The colour temperature on a bar from the light's warmest to its coolest.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Warmth {
    range: KelvinRange,
    shown: Shown<ColorTemperature>,
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
    position: Position,
    dimmer: Option<Dimmer>,
    led: Option<Color>,
}

impl Tile {
    /// `state` with what the user asked for laid over it.
    #[must_use]
    pub fn new(state: &State, context: &Context) -> Self {
        let pending = context.pending;
        let turning_on = [BRIGHTNESS, WARMTH, HUE]
            .into_iter()
            .any(|slot| pending.asked(slot));
        let position = pending
            .on(POWER)
            .or(turning_on.then_some(true))
            .map_or(state.position, |on| Position::of(Some(on)));
        let on = position == Position::On;
        let dimmer = state
            .dimming
            .map(|dimming| Dimmer::new(state, dimming, pending));
        let lit_brightness = dimmer
            .and_then(|dimmer| dimmer.brightness.value)
            .filter(|_| on);

        let room = context.room;
        let fits = card_view::rows((room.width, room.height), room.size);
        let candidates = [
            (Row::Brightness, dimmer.is_some()),
            (
                Row::Warmth,
                dimmer.is_some_and(|dimmer| dimmer.warmth.is_some()),
            ),
            (Row::Hue, dimmer.is_some_and(|dimmer| dimmer.hue.is_some())),
            (Row::Presets, dimmer.is_some()),
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
                lit_brightness.filter(|_| !rows.contains(&Row::Brightness)),
            ))),
            color: on
                .then(|| color.map_or(Accent::Amber, |color| Accent::Light(icon_color(color)))),
            icon: Icon(if on { &LIGHTBULB } else { &LIGHTBULB_OFF }),
            disc: if on { Disc::Tinted } else { Disc::Bare },
            mark: (position == Position::Unknown).then_some(Mark::Unknown),
        };
        Self {
            card: card(context, display, &card_action),
            card_action,
            rows,
            width: fits.width,
            can_act: context.can_act(),
            position,
            dimmer,
            led: match (lit_brightness, color.or_else(|| plain_white(state))) {
                (Some(brightness), Some(color)) => Some(led_color(color, brightness)),
                (None, _) | (_, None) => None,
            },
        }
    }

    fn is_on(&self) -> bool {
        self.position == Position::On
    }

    /// The brightness while on, as far as it is known.
    fn lit_brightness(&self) -> Option<Brightness> {
        self.dimmer?.brightness.value.filter(|_| self.is_on())
    }

    fn shows(&self, row: Row) -> bool {
        self.rows.contains(&row)
    }

    fn brightness(&self) -> Option<Stepper> {
        let shown = self
            .dimmer
            .filter(|_| self.shows(Row::Brightness))?
            .brightness;
        let brightness = self.lit_brightness();
        let on = self.is_on();
        Some(Stepper {
            key: BRIGHTNESS.name().to_owned(),
            ends: Ends::Signs,
            down: self.can_act && brightness.is_some(),
            up: self.can_act
                && (!on || brightness.is_some_and(|brightness| brightness < Brightness::FULL)),
            pending: shown.faint && brightness.is_some(),
            track: Track::Level {
                value: match (on, brightness) {
                    (false, _) => self.position.text().to_owned(),
                    (true, Some(brightness)) => percent_text(brightness),
                    (true, None) => UNKNOWN.to_owned(),
                },
                level: Level {
                    share: brightness.map_or(0.0, Brightness::share),
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
        let brightness = self.lit_brightness();
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
            selected: PERCENTS
                .iter()
                .position(|preset| Some(*preset) == brightness),
            role: Role::Options,
            width: self.width,
        })
    }

    fn warmth(&self) -> Option<Stepper> {
        let Warmth { range, shown } = self.dimmer?.warmth.filter(|_| self.shows(Row::Warmth))?;
        Some(Stepper {
            key: WARMTH.name().to_owned(),
            ends: Ends::Signs,
            down: self.can_act && shown.value.is_none_or(|kelvin| kelvin > range.warmest()),
            up: self.can_act && shown.value.is_none_or(|kelvin| kelvin < range.coolest()),
            pending: shown.faint,
            track: Track::Gradient(Gradient {
                stops: gradient::temperatures(
                    range.warmest().as_kelvin(),
                    range.coolest().as_kelvin(),
                ),
                share: shown.value.map(|kelvin| range.share_of(kelvin)),
            }),
            width: self.width,
        })
    }

    fn hue(&self) -> Option<Stepper> {
        let hue = self.dimmer?.hue.filter(|_| self.shows(Row::Hue))?;
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
        match (touched, self.is_on(), self.lit_brightness()) {
            (Touched::Set(share), _, _) => Some(set_brightness(
                Brightness::from_percent(percent_of(share)),
                Timing::Now,
            )),
            (Touched::Up, false, _) => Some(turn(true)),
            (Touched::Up, true, Some(brightness)) => Some(set_brightness(
                Brightness::from_percent((brightness.as_percent() / STEP + 1) * STEP),
                Timing::AfterTaps,
            )),
            (Touched::Down, true, Some(brightness)) => {
                let next = brightness.as_percent().saturating_sub(1) / STEP * STEP;
                Some(if next >= STEP {
                    set_brightness(Brightness::from_percent(next), Timing::AfterTaps)
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
        let Warmth { range, shown } = self.dimmer?.warmth?;
        let current = shown.value.map_or(0.5, |kelvin| range.share_of(kelvin));
        let (share, timing) = match touched {
            Touched::Set(share) => (share, Timing::Now),
            Touched::Up => (current + WARMTH_STEP, Timing::AfterTaps),
            Touched::Down => (current - WARMTH_STEP, Timing::AfterTaps),
        };
        let kelvin = range.at(share);
        Some(
            Request::number(
                WARMTH,
                kelvin.as_kelvin(),
                light::turn_on_at_kelvin(kelvin),
                timing,
            )
            .superseding(&[HUE, POWER]),
        )
    }

    /// A light that shows no hue steps from the middle of the circle.
    /// It drops a colour temperature still asked for, as a light shows one or the other.
    fn hue_after(&self, touched: Touched) -> Option<Request> {
        let current = self.dimmer?.hue?.value.unwrap_or(FULL_CIRCLE / 2.0);
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

    fn led(&self) -> Option<Color> {
        self.led
    }
}

/// The light's colour at full brightness, what the user asked for over what Home Assistant reports;
/// none for a light that tells neither a colour temperature nor a colour.
fn light_color(state: &State, pending: &Pending) -> Option<Color> {
    match (pending.number(HUE), pending.number(WARMTH)) {
        (Some(hue), _) => Some(hue_color(hue, narrow(SATURATION))),
        (None, Some(kelvin)) => Some(gradient::temperature_color(kelvin)),
        (None, None) => match state.look {
            Look::White(kelvin) => Some(gradient::temperature_color(kelvin.as_kelvin())),
            Look::Colour(colour) => Some(hue_color(narrow(colour.hue), narrow(colour.saturation))),
            Look::Plain | Look::Unknown => None,
        },
    }
}

/// A warm white for a light whose white has no stated temperature:
/// one that takes neither a colour temperature nor a colour,
/// or one Home Assistant reports lit in a plain white.
/// Any other light comes back in a look the tile learns only from Home Assistant's report.
fn plain_white(state: &State) -> Option<Color> {
    let takes_neither = matches!(
        state.dimming,
        Some(Dimming {
            warmth: None,
            colour: false
        })
    );
    let lit_in_plain_white = state.look == Look::Plain;
    if takes_neither || lit_in_plain_white {
        Some(gradient::temperature_color(PLAIN_KELVIN.as_kelvin()))
    } else {
        None
    }
}

/// The strip's colour: the light's, as bright as the light.
fn led_color(color: Color, brightness: Brightness) -> Color {
    color.brightness(brightness.share().max(LED_FLOOR))
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
fn line(position: Position, brightness: Option<Brightness>) -> String {
    match (position, brightness) {
        (Position::On, Some(brightness)) => percent_text(brightness),
        (position, _) => position.text().to_owned(),
    }
}

/// The light turned on at `brightness`.
fn set_brightness(brightness: Brightness, timing: Timing) -> Request {
    Request::number(
        BRIGHTNESS,
        f32::from(brightness.as_percent()),
        light::turn_on_at_percent(brightness),
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

/// A brightness asked for, as the pending request holds it.
fn brightness_of(number: f32) -> Brightness {
    Brightness::from_percent(percent_of(number / 100.0))
}

fn percent_text(brightness: Brightness) -> String {
    let percent = brightness.as_percent();
    with_unit(fmt!("{percent}"), Some("%"))
}

#[cfg(test)]
mod tests;

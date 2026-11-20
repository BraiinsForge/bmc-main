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

//! A value between two buttons that step it, over a bar a tap sets where the value has two ends.

use bmc_wasm_sdk::*;

use super::control_button::{ControlButton, Face, Look};
use super::gradient;
use crate::view::icons::{ARROW_LEFT, ARROW_RIGHT};
use crate::view::style::{
    CONTROL_HEIGHT, CONTROL_RADIUS, ICON_SIZE, Icon, LABEL_COLOR, PENDING_LABEL_COLOR, TRACK_COLOR,
    dimmed_unless, fill_insets, fitted_text, row_spacing,
};
use crate::view::touches::{Touches, middle, part_key};

const DOWN: &str = "down";
const UP: &str = "up";
const TRACK: &str = "track";
const SIGN_SIZE: u32 = 32;
const VALUE_SIZE: u32 = 28;
const LEVEL_FILL: f32 = 0.35;
const PENDING_FILL: f32 = 0.18;

/// What the buttons at the stepper's ends show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ends {
    /// − and +, for a number.
    Signs,
    /// ← and →, for one of a list of options.
    Arrows,
}

/// What the track between the buttons shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Track {
    /// The value alone, for a value without two ends, such as a thermostat's target.
    Value(String),
    /// The value over a bar filled to it, such as a brightness;
    /// a tap sets the value where the bar would end.
    Level { value: String, level: Level },
    /// Colours from end to end and a cursor at the value, without a number to read,
    /// such as a hue; a tap picks the colour under it.
    Gradient(Gradient),
}

/// The bar a value with two ends sits on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    /// How far the value is between its ends, from 0 to 1.
    pub share: f32,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    /// Spread evenly from end to end.
    pub stops: Vec<Color>,
    /// Where the cursor stands, from 0 to 1;
    /// none without a value to show, such as a colour temperature while the light shows a colour.
    pub share: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stepper {
    pub key: String,
    pub ends: Ends,
    /// Stepping down has something to do; the button is dimmed and does nothing otherwise.
    pub down: bool,
    pub up: bool,
    /// Home Assistant has yet to confirm the value: it is drawn fainter, its bar lighter, its cursor's grip grey.
    pub pending: bool,
    pub track: Track,
    pub width: f32,
}

/// A touch on a [`Stepper`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touched {
    Down,
    Up,
    /// The bar or the gradient, at `share` of the way between its ends, from 0 to 1.
    Set(f32),
}

impl Stepper {
    /// With nothing to step either way, the value is dimmed too, and a tap on the track does nothing.
    #[must_use]
    pub fn view(&self, touches: &Touches) -> Node {
        let (button, gap) = row_spacing(self.width);
        let track_width = self.track_width();
        let value_color = if self.pending {
            PENDING_LABEL_COLOR
        } else {
            LABEL_COLOR
        };
        let mut track = Vec::new();
        match &self.track {
            Track::Value(value) => track.push(value_text(value, track_width, value_color)),
            Track::Level { value, level } => {
                // A level of 0 shows no bar rather than the bar's rounded ends.
                if level.share > 0.0 {
                    let fill = if self.pending {
                        PENDING_FILL
                    } else {
                        LEVEL_FILL
                    };
                    track.push(level_bar(
                        level.share,
                        track_width,
                        level.color.with_alpha(fill),
                    ));
                }
                track.push(value_text(value, track_width, value_color));
            }
            Track::Gradient(gradient) => track.push(canvas(
                fill_insets(),
                gradient::paint(
                    &gradient.stops,
                    gradient.share,
                    (track_width, CONTROL_HEIGHT),
                    self.pending,
                ),
            )),
        }
        track.push(touchable(
            &part_key(&self.key, TRACK),
            fill_insets(),
            Vec::<Draw>::new(),
        ));
        let [down, up] = self.ends(button);
        row(
            props!(gap: gap),
            [
                down.view(touches),
                dimmed_unless(
                    self.live(),
                    col(
                        props!(flex: 1.0, height: CONTROL_HEIGHT, background: TRACK_COLOR, border_radius: CONTROL_RADIUS),
                        track,
                    ),
                ),
                up.view(touches),
            ],
        )
    }

    #[must_use]
    pub fn touched(&self, touches: &Touches) -> Option<Touched> {
        let (part, hit) = touches.released_part(&self.key)?;
        match part {
            DOWN if self.down => Some(Touched::Down),
            UP if self.up => Some(Touched::Up),
            TRACK if self.live() => match &self.track {
                Track::Value(_) => None,
                Track::Level { .. } => Some(Touched::Set(level_share_at(hit))),
                Track::Gradient(_) => Some(Touched::Set(gradient::share_at(hit.x, hit.width))),
            },
            _ => None,
        }
    }

    /// The touches of a finger lifted where it makes `touched`; none for a value without a track to tap.
    #[must_use]
    pub fn tap(&self, touched: Touched) -> Touches {
        self.target(touched)
            .map_or_else(Touches::default, |(key, hit)| Touches::tap(&key, hit))
    }

    /// The touches of a finger on the part `touched` names.
    #[must_use]
    pub fn hold(&self, touched: Touched) -> Touches {
        self.target(touched)
            .map_or_else(Touches::default, |(key, hit)| Touches::hold(&key, hit))
    }

    fn target(&self, touched: Touched) -> Option<(String, TouchHit)> {
        let (button, _) = row_spacing(self.width);
        let width = self.track_width();
        let x = match (touched, &self.track) {
            (Touched::Down, _) => return Some(self.part(DOWN, middle(button, CONTROL_HEIGHT))),
            (Touched::Up, _) => return Some(self.part(UP, middle(button, CONTROL_HEIGHT))),
            (Touched::Set(_), Track::Value(_)) => return None,
            (Touched::Set(share), Track::Level { .. }) => bar_width(share, width),
            (Touched::Set(share), Track::Gradient(_)) => gradient::cursor_x(share, width),
        };
        Some(self.part(
            TRACK,
            TouchHit {
                x,
                ..middle(width, CONTROL_HEIGHT)
            },
        ))
    }

    fn part(&self, part: &str, hit: TouchHit) -> (String, TouchHit) {
        (part_key(&self.key, part), hit)
    }

    fn live(&self) -> bool {
        self.down || self.up
    }

    fn track_width(&self) -> f32 {
        let (button, gap) = row_spacing(self.width);
        (self.width - 2.0 * (button + gap)).max(0.0)
    }

    fn ends(&self, width: f32) -> [ControlButton; 2] {
        let faces = match self.ends {
            Ends::Signs => ["\u{2212}", "+"].map(|sign| Face::Text {
                text: sign.to_owned(),
                size: SIGN_SIZE,
            }),
            Ends::Arrows => [&ARROW_LEFT, &ARROW_RIGHT].map(|svg| Face::Icon {
                icon: Icon(svg),
                size: ICON_SIZE,
            }),
        };
        let [down, up] = faces;
        [(DOWN, down, self.down), (UP, up, self.up)].map(|(part, face, enabled)| ControlButton {
            key: part_key(&self.key, part),
            face,
            look: Look::Plain,
            width,
            height: CONTROL_HEIGHT,
            enabled,
        })
    }
}

fn value_text(value: &str, width: f32, color: Color) -> Node {
    fitted_text(
        value,
        (width, CONTROL_HEIGHT),
        CONTROL_HEIGHT,
        VALUE_SIZE,
        color,
    )
}

/// The narrowest a level bar gets: its two rounded ends.
const BAR_ENDS: f32 = 2.0 * CONTROL_RADIUS;

/// A bar filling `share` of a `width` wide track from the left, behind its value.
fn level_bar(share: f32, width: f32, color: Color) -> Node {
    row(
        fill_insets(),
        [col(
            props!(width: bar_width(share, width), background: color, border_radius: CONTROL_RADIUS),
            [],
        )],
    )
}

/// How wide a bar filling `share` of a `width` wide track is drawn.
/// It is never narrower than its two rounded ends, which a narrower bar draws out of shape.
fn bar_width(share: f32, width: f32) -> f32 {
    BAR_ENDS + share.clamp(0.0, 1.0) * (width - BAR_ENDS).max(0.0)
}

/// The share a tap at `hit` on the bar sets, [`bar_width`] turned around: the bar then ends under the finger.
fn level_share_at(hit: &TouchHit) -> f32 {
    let span = hit.width - BAR_ENDS;
    if span > 0.0 {
        ((hit.x - BAR_ENDS) / span).clamp(0.0, 1.0)
    } else {
        hit.frac_x()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    const WIDTH: f32 = 400.0;

    fn stepper(track: Track) -> Stepper {
        Stepper {
            key: "c1".to_owned(),
            ends: Ends::Signs,
            down: true,
            up: true,
            pending: false,
            track,
            width: WIDTH,
        }
    }

    fn level() -> Stepper {
        stepper(Track::Level {
            value: "60%".to_owned(),
            level: Level {
                share: 0.6,
                color: WHITE,
            },
        })
    }

    fn tap_track_at(stepper: &Stepper, x: f32) -> Touches {
        Touches::tap(
            "c1:track",
            TouchHit {
                x,
                ..middle(stepper.track_width(), CONTROL_HEIGHT)
            },
        )
    }

    #[wasm_test]
    fn each_touch_reads_back_as_itself() {
        let gradient = stepper(Track::Gradient(Gradient {
            stops: vec![WHITE, BLACK],
            share: Some(0.5),
        }));
        for stepper in [level(), gradient] {
            for touched in [Touched::Down, Touched::Up, Touched::Set(0.25)] {
                assert_eq!(
                    stepper.touched(&stepper.tap(touched)),
                    Some(touched),
                    "{:?}",
                    stepper.track
                );
            }
        }
    }

    #[wasm_test]
    fn a_tap_on_the_bar_sets_the_share_whose_bar_ends_under_the_finger() {
        let stepper = level();
        let share = |x| match stepper.touched(&tap_track_at(&stepper, x)) {
            Some(Touched::Set(share)) => share,
            other => panic!("not a set: {other:?}"),
        };
        assert_eq!(share(BAR_ENDS), 0.0, "the narrowest bar");
        assert_eq!(share(stepper.track_width()), 1.0, "the full bar");
        assert_eq!(share(4.0), 0.0, "within the rounded ends");
    }

    #[wasm_test]
    fn a_value_without_ends_sets_nothing() {
        let stepper = stepper(Track::Value("21.5 °C".to_owned()));
        assert_eq!(stepper.touched(&tap_track_at(&stepper, 50.0)), None);
        assert!(stepper.tap(Touched::Set(0.5)).released_part("c1").is_none());
    }

    #[wasm_test]
    fn an_end_with_nothing_to_do_takes_its_touch_and_does_nothing() {
        let stepper = Stepper {
            down: false,
            ..level()
        };
        assert_eq!(stepper.touched(&stepper.tap(Touched::Down)), None);
        assert_eq!(
            stepper.touched(&stepper.tap(Touched::Up)),
            Some(Touched::Up)
        );
    }

    #[wasm_test]
    fn with_nothing_to_step_the_track_does_nothing_either() {
        let stepper = Stepper {
            down: false,
            up: false,
            ..level()
        };
        assert_eq!(stepper.touched(&stepper.tap(Touched::Set(0.5))), None);
    }

    #[wasm_test]
    fn another_blocks_touches_are_none_of_this_ones() {
        let stepper = level();
        for other in ["c2:up", "c10:up", "c1", "card"] {
            assert_eq!(
                stepper.touched(&Touches::tap(other, middle(64.0, 64.0))),
                None,
                "{other}"
            );
        }
    }
}

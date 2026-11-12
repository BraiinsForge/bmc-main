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

//! The keys a code is typed on, laid out as on a phone, the key that cancels it,
//! and the dots that stand for the digits typed.

use bmc_wasm_sdk::*;

use super::control_button::{ControlButton, Face, Look};
use crate::view::icons::{BACKSPACE_OUTLINE, CHECK, CLOSE};
use crate::view::style::{CONTROL_HEIGHT, Icon};
use crate::view::touches::{Touches, part_key};

const BACK: &str = "back";
const OK: &str = "ok";
const CANCEL: &str = "cancel";
const GAP: f32 = 12.0;
const DIGIT_SIZE: u32 = 40;
const ICON_SIZE: f32 = 36.0;
const ROWS: [[Key; 3]; 4] = [
    [
        Key::Digit(Digit(1)),
        Key::Digit(Digit(2)),
        Key::Digit(Digit(3)),
    ],
    [
        Key::Digit(Digit(4)),
        Key::Digit(Digit(5)),
        Key::Digit(Digit(6)),
    ],
    [
        Key::Digit(Digit(7)),
        Key::Digit(Digit(8)),
        Key::Digit(Digit(9)),
    ],
    [Key::Back, Key::Digit(Digit(0)), Key::Ok],
];
const LABELS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
/// A dot, and the distance between dots, as shares of the line's font size.
const DOT_RADIUS: f32 = 0.3;
const DOT_PITCH: f32 = 0.9;
/// A line of text, as a share of its font size, with room for descenders.
const LINE: f32 = 1.2;

/// A key on the [`Keypad`], or the one that cancels the code beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Digit(Digit),
    Back,
    Ok,
    Cancel,
}

/// A digit from 0 to 9, which only [`Digit::ALL`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Digit(u8);

impl Digit {
    pub const ALL: [Self; 10] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
        Self(8),
        Self(9),
    ];

    fn label(self) -> &'static str {
        LABELS[usize::from(self.0)]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keypad {
    pub key: String,
    /// Back and OK have something to do only once a digit is typed.
    pub typed: bool,
    /// The colour OK wears once it has something to do.
    pub color: Color,
    pub width: f32,
    pub row_height: f32,
}

impl Keypad {
    /// The digits, Back and OK, without the key that cancels: see [`Self::cancel`].
    #[must_use]
    pub fn view(&self, touches: &Touches) -> Node {
        col(
            props!(width: self.width, gap: GAP),
            ROWS.map(|keys| {
                row(
                    props!(gap: GAP),
                    keys.map(|key| self.button(key).view(touches)),
                )
            }),
        )
    }

    /// The ✕ that cancels the code, which stands apart from the other keys.
    #[must_use]
    pub fn cancel(&self, touches: &Touches) -> Node {
        self.button(Key::Cancel).view(touches)
    }

    /// The key a finger was lifted from.
    #[must_use]
    pub fn touched(&self, touches: &Touches) -> Option<Key> {
        let (part, _) = touches.released_part(&self.key)?;
        match part {
            BACK => self.typed.then_some(Key::Back),
            OK => self.typed.then_some(Key::Ok),
            CANCEL => Some(Key::Cancel),
            label => Digit::ALL
                .into_iter()
                .find(|digit| digit.label() == label)
                .map(Key::Digit),
        }
    }

    /// The touches of a finger lifted from `key`.
    #[must_use]
    pub fn tap(&self, key: Key) -> Touches {
        self.button(key).tap()
    }

    /// The touches of a finger on `key`.
    #[must_use]
    pub fn hold(&self, key: Key) -> Touches {
        self.button(key).hold()
    }

    fn button(&self, key: Key) -> ControlButton {
        let (part, face, look, enabled) = match key {
            Key::Digit(digit) => {
                let digit = digit.label();
                let face = Face::Text {
                    text: digit.to_owned(),
                    size: DIGIT_SIZE,
                };
                (digit, face, Look::Plain, true)
            }
            Key::Back => (BACK, icon(&BACKSPACE_OUTLINE), Look::Plain, self.typed),
            Key::Ok => (
                OK,
                icon(&CHECK),
                if self.typed {
                    Look::Accent(self.color)
                } else {
                    Look::Plain
                },
                self.typed,
            ),
            Key::Cancel => (CANCEL, icon(&CLOSE), Look::Plain, true),
        };
        let (width, height) = match key {
            Key::Cancel => (CONTROL_HEIGHT, CONTROL_HEIGHT),
            Key::Digit(_) | Key::Back | Key::Ok => {
                (((self.width - 2.0 * GAP) / 3.0).max(0.0), self.row_height)
            }
        };
        ControlButton {
            key: part_key(&self.key, part),
            face,
            look,
            width,
            height,
            enabled,
        }
    }
}

fn icon(svg: &'static Svg) -> Face {
    Face::Icon {
        icon: Icon(svg),
        size: ICON_SIZE,
    }
}

/// `line`, a line of text `size` high, followed by `dots` dots, as a code typed shows;
/// drawn rather than typed, as the font has no large dot.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a code is a handful of digits, and a font size a few dozen px"
)]
pub fn with_dots(line: Node, size: u32, dots: usize, color: Color) -> Node {
    if dots == 0 {
        return line;
    }
    let size = size as f32;
    let (radius, pitch) = (size * DOT_RADIUS, size * DOT_PITCH);
    let height = size * LINE;
    row(
        props!(gap: pitch - 2.0 * radius, cross_align: CrossAlign::Center),
        [
            line,
            canvas(
                props!(width: dots as f32 * pitch, height: height),
                (0..dots)
                    .map(|dot| {
                        Draw::circle(radius + dot as f32 * pitch, height / 2.0, radius, color)
                            .with_anti_alias()
                    })
                    .collect::<Vec<_>>(),
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::touches::middle;
    use bmc_wasm_sdk::wasm_test;

    fn keypad(typed: bool) -> Keypad {
        Keypad {
            key: "pad".to_owned(),
            typed,
            color: WHITE,
            width: 400.0,
            row_height: 64.0,
        }
    }

    #[wasm_test]
    fn every_key_reads_back_as_itself() {
        let keypad = keypad(true);
        let keys = Digit::ALL
            .map(Key::Digit)
            .into_iter()
            .chain([Key::Back, Key::Ok, Key::Cancel]);
        for key in keys {
            assert_eq!(keypad.touched(&keypad.tap(key)), Some(key));
        }
        for other in ["pad:10", "pad:", "pad", "card", "c0:up"] {
            let touches = Touches::tap(other, middle(64.0, 64.0));
            assert_eq!(keypad.touched(&touches), None, "{other}");
        }
    }

    #[wasm_test]
    fn before_a_digit_back_and_ok_do_nothing() {
        let keypad = keypad(false);
        assert_eq!(keypad.touched(&keypad.tap(Key::Back)), None);
        assert_eq!(keypad.touched(&keypad.tap(Key::Ok)), None);
        assert_eq!(keypad.touched(&keypad.tap(Key::Cancel)), Some(Key::Cancel));
    }
}

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

//! The frontend's `ha-control-button`.

use bmc_wasm_sdk::*;

use crate::view::style::{
    CONTROL_RADIUS, Icon, LABEL_COLOR, TRACK_COLOR, dimmed_unless, fill_insets, fitted_text,
};
use crate::view::touches::{Touches, middle};

/// An accented button's fill, idle and pressed.
const ACCENT_FILL: f32 = 0.35;
const ACCENT_PRESSED_FILL: f32 = 0.5;
const UNPICKED_ICON_COLOR: Color = GRAY_40;
/// Between a fitted label and the button's sides.
const FITTED_INSET: f32 = 8.0;

/// What a button shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Face {
    /// One line of text at its size, such as − or a digit.
    Text {
        text: String,
        size: u32,
    },
    /// Text shrunk to fit the button's width, on a line `line` high, such as a chip's label.
    Fitted {
        text: String,
        size: u32,
        line: f32,
    },
    Icon {
        icon: Icon,
        size: f32,
    },
}

/// What the button stands for, which picks its colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Look {
    /// A command, such as + or a key.
    Plain,
    /// An option not chosen: its icon greyed, so the chosen one stands out.
    Unpicked,
    /// The option chosen, where the option has no colour of its own.
    Picked,
    /// Tinted with a state's colour: the option chosen, or the card's own action.
    /// Its icon wears the colour, while text stays white to stay readable.
    Accent(Color),
}

impl Look {
    /// The background idle and while pressed, a step brighter.
    fn backgrounds(self) -> [Color; 2] {
        match self {
            Self::Plain | Self::Unpicked => [TRACK_COLOR, GRAY_80],
            Self::Picked => [GRAY_70, GRAY_60],
            Self::Accent(color) => {
                [ACCENT_FILL, ACCENT_PRESSED_FILL].map(|fill| color.with_alpha(fill))
            }
        }
    }

    fn content(self, face: &Face) -> Color {
        match (self, face) {
            (Self::Unpicked, Face::Icon { .. }) => UNPICKED_ICON_COLOR,
            (Self::Accent(color), Face::Icon { .. }) => color,
            (Self::Plain | Self::Unpicked | Self::Picked | Self::Accent(_), _) => LABEL_COLOR,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlButton {
    pub key: String,
    pub face: Face,
    pub look: Look,
    pub width: f32,
    pub height: f32,
    /// A button with nothing to do now is dimmed, and its touches do nothing;
    /// it still takes them, so a finger on it never reaches what lies under it.
    pub enabled: bool,
}

impl ControlButton {
    /// Pressed while a finger is on it.
    #[must_use]
    pub fn view(&self, touches: &Touches) -> Node {
        let [idle, pressed] = self.look.backgrounds();
        let layers = [
            self.face(),
            touchable(&self.key, fill_insets(), Vec::<Draw>::new()),
        ];
        dimmed_unless(
            self.enabled,
            col(
                props!(
                    width: self.width,
                    height: self.height,
                    background: if self.held(touches) { pressed } else { idle },
                    border_radius: CONTROL_RADIUS
                ),
                layers,
            ),
        )
    }

    /// A finger was lifted from it.
    #[must_use]
    pub fn touched(&self, touches: &Touches) -> bool {
        self.enabled && touches.released(&self.key).is_some()
    }

    fn held(&self, touches: &Touches) -> bool {
        self.enabled && touches.is_held(&self.key)
    }

    /// The touches of a finger lifted from it.
    #[must_use]
    pub fn tap(&self) -> Touches {
        Touches::tap(&self.key, middle(self.width, self.height))
    }

    /// The touches of a finger on it.
    #[must_use]
    pub fn hold(&self) -> Touches {
        Touches::hold(&self.key, middle(self.width, self.height))
    }

    fn face(&self) -> Node {
        let color = self.look.content(&self.face);
        match &self.face {
            Face::Text { text: value, size } => center(
                props!(flex: 1.0),
                [text(
                    value.clone(),
                    style!(size: *size, weight: FontWeight::BOLD, color: color, line_height: 1.0),
                )],
            ),
            Face::Fitted { text, size, line } => fitted_text(
                text,
                ((self.width - 2.0 * FITTED_INSET).max(0.0), *line),
                self.height,
                *size,
                color,
            ),
            Face::Icon { icon, size } => center(
                props!(flex: 1.0),
                [canvas(
                    props!(width: *size, height: *size),
                    [Draw::svg(0.0, 0.0, *size, *size, icon.0, color).with_anti_alias()],
                )],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    fn button(enabled: bool) -> ControlButton {
        ControlButton {
            key: "press".to_owned(),
            face: Face::Text {
                text: "Press".to_owned(),
                size: 28,
            },
            look: Look::Plain,
            width: 200.0,
            height: 64.0,
            enabled,
        }
    }

    #[wasm_test]
    fn a_tap_on_a_button_reads_back_as_touched() {
        let button = button(true);
        assert!(button.touched(&button.tap()));
        assert!(
            !button.touched(&button.hold()),
            "not until the finger lifts"
        );
        assert!(!button.touched(&Touches::default()));
    }

    #[wasm_test]
    fn a_disabled_button_does_nothing_with_its_touches() {
        let button = button(false);
        assert!(!button.touched(&button.tap()));
        assert!(!button.held(&button.hold()));
    }
}

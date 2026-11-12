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

//! Options or commands side by side as buttons sharing a row, as many as there is room for.

use bmc_wasm_sdk::*;

use super::control_button::{ControlButton, Face, Look};
use crate::view::style::{CONTROL_HEIGHT, ICON_SIZE, Icon, row_spacing};
use crate::view::touches::{Touches, middle, part_key};

const OPTION: &str = "option";
const ICON_CHIP_MIN: f32 = 52.0;
/// Three of these fit a small tile's row.
const LABEL_CHIP_MIN: f32 = 80.0;
const LABEL_SIZE: u32 = 22;
/// Room for one line of a label and never a second, so a long label shrinks rather than breaking a word.
const LABEL_LINE: f32 = 28.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub label: String,
    /// Drawn in place of the label where every chip of the row has one.
    pub icon: Option<Icon>,
    /// The colour the chip wears while selected, such as a thermostat mode's; none for a neutral one.
    pub accent: Option<Color>,
    /// A chip with nothing to do now, such as Open on a cover that is open, is dimmed and its touches do nothing.
    pub enabled: bool,
}

/// What the chips of a row stand for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// One of them is the value, shown selected.
    Options,
    /// Things to do: never selected, their icons as bright as a label.
    Commands,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chips {
    pub key: String,
    pub chips: Vec<Chip>,
    pub selected: Option<usize>,
    pub role: Role,
    pub width: f32,
}

impl Chips {
    /// Icons where every chip has one, else the labels, shrunk to fit.
    #[must_use]
    pub fn view(&self, touches: &Touches) -> Node {
        let (_, gap) = row_spacing(self.width);
        let icons: Option<Vec<Icon>> = self.chips.iter().map(|chip| chip.icon).collect();
        let min = if icons.is_some() {
            ICON_CHIP_MIN
        } else {
            LABEL_CHIP_MIN
        };
        let shown = fitting(self.chips.len(), self.selected, self.width, gap, min);
        #[expect(
            clippy::cast_precision_loss,
            reason = "a handful of chips across a widget"
        )]
        let chip_width = (self.width - gap * (shown.len() as f32 - 1.0)) / shown.len() as f32;
        row(
            props!(gap: gap),
            shown.into_iter().map(|option| {
                let chip = &self.chips[option];
                ControlButton {
                    key: self.option_key(option),
                    face: match &icons {
                        Some(icons) => Face::Icon {
                            icon: icons[option],
                            size: ICON_SIZE,
                        },
                        None => Face::Fitted {
                            text: chip.label.clone(),
                            size: LABEL_SIZE,
                            line: LABEL_LINE,
                        },
                    },
                    look: self.look(option),
                    width: chip_width,
                    height: CONTROL_HEIGHT,
                    enabled: chip.enabled,
                }
                .view(touches)
            }),
        )
    }

    /// The chip a finger was lifted from.
    #[must_use]
    pub fn touched(&self, touches: &Touches) -> Option<usize> {
        let (part, _) = touches.released_part(&self.key)?;
        let option: usize = part.strip_prefix(OPTION)?.parse().ok()?;
        self.chips
            .get(option)
            .is_some_and(|chip| chip.enabled)
            .then_some(option)
    }

    /// The touches of a finger lifted from the chip `option`.
    #[must_use]
    pub fn tap(&self, option: usize) -> Touches {
        Touches::tap(&self.option_key(option), middle(self.width, CONTROL_HEIGHT))
    }

    /// The touches of a finger on the chip `option`.
    #[must_use]
    pub fn hold(&self, option: usize) -> Touches {
        Touches::hold(&self.option_key(option), middle(self.width, CONTROL_HEIGHT))
    }

    fn option_key(&self, option: usize) -> String {
        part_key(&self.key, &fmt!("{OPTION}{option}"))
    }

    fn look(&self, option: usize) -> Look {
        match (self.selected == Some(option), self.chips[option].accent) {
            (true, Some(color)) => Look::Accent(color),
            (true, None) => Look::Picked,
            (false, _) => match self.role {
                Role::Commands => Look::Plain,
                Role::Options => Look::Unpicked,
            },
        }
    }
}

/// The chips that fit `width` at least `min` wide each: all of them,
/// or the selected one and the first others.
fn fitting(count: usize, selected: Option<usize>, width: f32, gap: f32, min: f32) -> Vec<usize> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a positive count of chips a few hundred px wide"
    )]
    let room = (((width + gap) / (min + gap)).floor().max(1.0)) as usize;
    if count <= room {
        return (0..count).collect();
    }
    let mut kept: Vec<usize> = selected
        .filter(|option| *option < count)
        .into_iter()
        .collect();
    kept.extend(
        (0..count)
            .filter(|option| Some(*option) != selected)
            .take(room - kept.len()),
    );
    kept.sort_unstable();
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    fn chips() -> Chips {
        Chips {
            key: "c2".to_owned(),
            chips: ["Off", "Heat", "Cool", "Auto"]
                .map(|label| Chip {
                    label: label.to_owned(),
                    icon: None,
                    accent: None,
                    enabled: label != "Cool",
                })
                .to_vec(),
            selected: None,
            role: Role::Options,
            width: 400.0,
        }
    }

    #[wasm_test]
    fn a_tap_on_a_chip_reads_back_as_its_index() {
        let chips = chips();
        assert_eq!(chips.touched(&chips.tap(3)), Some(3));
        assert_eq!(chips.touched(&chips.tap(2)), None, "a dimmed chip");
        assert_eq!(chips.touched(&chips.tap(4)), None, "no such chip");
        for other in ["c2:option", "c2:optionx", "c3:option1", "c2:up", "card"] {
            let touches = Touches::tap(other, middle(80.0, 64.0));
            assert_eq!(chips.touched(&touches), None, "{other}");
        }
    }

    #[wasm_test]
    fn chips_that_do_not_fit_leave_room_for_the_selected_one() {
        assert_eq!(
            fitting(3, Some(2), 600.0, 12.0, ICON_CHIP_MIN),
            vec![0, 1, 2]
        );
        assert_eq!(
            fitting(7, Some(6), 200.0, 8.0, ICON_CHIP_MIN),
            vec![0, 1, 6],
            "three fit, the selected one among them"
        );
        assert_eq!(fitting(7, None, 200.0, 8.0, ICON_CHIP_MIN), vec![0, 1, 2]);
    }
}

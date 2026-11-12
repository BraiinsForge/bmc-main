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

//! The frontend's `ha-control-switch`: a rounded track holding "O" and "|", or two icons, and a thumb over the current half.

use std::f32::consts::TAU;

use bmc_wasm_sdk::*;

use crate::view::style::{
    CONTROL_RADIUS, Icon, LABEL_COLOR, TRACK_COLOR, dimmed_unless, fill_insets,
};
use crate::view::touches::{Touches, middle};

const THUMB_INSET: f32 = 6.0;
const THUMB_OFF_COLOR: Color = GRAY_70;
const SYMBOL_DIM_COLOR: Color = GRAY_50;
const ICON_SHARE: f32 = 0.5;
const ICON_MAX: f32 = 48.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ControlSwitch {
    pub key: String,
    pub on: bool,
    /// Drawn for off and on in place of "O" and "|".
    pub icons: Option<[Icon; 2]>,
    /// A switch that cannot be flipped now is dimmed, and its touches do nothing.
    pub enabled: bool,
    /// The thumb's colour while on.
    pub color: Color,
    pub width: f32,
    pub height: f32,
}

impl ControlSwitch {
    /// Shows no press: the flip a tap asks for is what answers it.
    #[must_use]
    pub fn view(&self) -> Node {
        let height = self.height;
        let half = (self.width - 2.0 * THUMB_INSET) / 2.0;
        let (off_center, on_center) = (THUMB_INSET + half / 2.0, THUMB_INSET + half * 1.5);
        let middle = height / 2.0;
        let stroke = (height * 0.05).max(3.0);
        let (off_symbol, on_symbol) = if self.on {
            (SYMBOL_DIM_COLOR, LABEL_COLOR)
        } else {
            (LABEL_COLOR, SYMBOL_DIM_COLOR)
        };
        let thumb = col(
            props!(
                flex: 1.0,
                background: if self.on { self.color } else { THUMB_OFF_COLOR },
                border_radius: CONTROL_RADIUS - THUMB_INSET / 2.0
            ),
            [],
        );
        let symbols = match self.icons {
            Some([off_icon, on_icon]) => [
                (off_center, off_icon, off_symbol),
                (on_center, on_icon, on_symbol),
            ]
            .map(|(center, icon, color)| {
                let side = (height * ICON_SHARE).min(ICON_MAX);
                Draw::svg(
                    center - side / 2.0,
                    middle - side / 2.0,
                    side,
                    side,
                    icon.0,
                    color,
                )
                .with_anti_alias()
            })
            .to_vec(),
            None => vec![
                Draw::arc(
                    off_center,
                    middle,
                    height * 0.14,
                    0.0,
                    TAU,
                    stroke,
                    off_symbol,
                    ArcSegments::Continuous,
                    ArcCap::Round,
                ),
                Draw::rect(
                    on_center - stroke / 2.0,
                    middle - height * 0.15,
                    stroke,
                    height * 0.3,
                    on_symbol,
                ),
            ],
        };
        let thumb_row = if self.on {
            vec![spacer(1.0), thumb]
        } else {
            vec![thumb, spacer(1.0)]
        };
        dimmed_unless(
            self.enabled,
            col(
                props!(
                    width: self.width,
                    height: height,
                    background: TRACK_COLOR,
                    border_radius: CONTROL_RADIUS
                ),
                [
                    row(
                        props!(
                            inset_top: THUMB_INSET,
                            inset_right: THUMB_INSET,
                            inset_bottom: THUMB_INSET,
                            inset_left: THUMB_INSET
                        ),
                        thumb_row,
                    ),
                    canvas(fill_insets(), symbols),
                    touchable(&self.key, fill_insets(), Vec::<Draw>::new()),
                ],
            ),
        )
    }

    /// A finger was lifted from it, which asks for it flipped.
    #[must_use]
    pub fn touched(&self, touches: &Touches) -> bool {
        self.enabled && touches.released(&self.key).is_some()
    }

    /// The touches of a finger lifted from it.
    #[must_use]
    pub fn tap(&self) -> Touches {
        Touches::tap(&self.key, middle(self.width, self.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    fn switch(enabled: bool) -> ControlSwitch {
        ControlSwitch {
            key: "power".to_owned(),
            on: false,
            icons: None,
            enabled,
            color: WHITE,
            width: 200.0,
            height: 64.0,
        }
    }

    #[wasm_test]
    fn a_tap_on_a_switch_reads_back_as_touched() {
        let switch = switch(true);
        assert!(switch.touched(&switch.tap()));
        assert!(!switch.touched(&Touches::default()));
    }

    #[wasm_test]
    fn a_disabled_switch_does_nothing_with_its_touches() {
        let switch = switch(false);
        assert!(!switch.touched(&switch.tap()));
    }
}

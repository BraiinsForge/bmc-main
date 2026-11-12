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

//! The button that pages to the next of several controls sharing one place,
//! showing that control's icon over a dot for each page, the current one lit.

use bmc_wasm_sdk::*;

use crate::view::style::{
    CONTROL_HEIGHT, CONTROL_RADIUS, ICON_SIZE, Icon, LABEL_COLOR, TRACK_COLOR, fill_insets,
};
use crate::view::touches::{Touches, middle};

const DOT_RADIUS: f32 = 3.5;
const DOT_SPACING: f32 = 12.0;
const DOT_INSET: f32 = 12.0;
const ICON_TOP: f32 = 8.0;
const DOT_OFF_COLOR: Color = GRAY_50;
const PRESSED_COLOR: Color = GRAY_80;

#[derive(Debug, Clone, PartialEq)]
pub struct PageButton {
    pub key: String,
    /// The icon of the control a touch pages to.
    pub icon: Icon,
    pub page: usize,
    pub count: usize,
    pub side: f32,
}

impl PageButton {
    /// Pressed while a finger is on it.
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "a handful of pages")]
    pub fn view(&self, touches: &Touches) -> Node {
        let first = self.side / 2.0 - self.count.saturating_sub(1) as f32 * DOT_SPACING / 2.0;
        let mut draws = vec![
            Draw::svg(
                (self.side - ICON_SIZE) / 2.0,
                ICON_TOP,
                ICON_SIZE,
                ICON_SIZE,
                self.icon.0,
                LABEL_COLOR,
            )
            .with_anti_alias(),
        ];
        draws.extend((0..self.count).map(|page| {
            let color = if page == self.page {
                LABEL_COLOR
            } else {
                DOT_OFF_COLOR
            };
            Draw::circle(
                first + page as f32 * DOT_SPACING,
                CONTROL_HEIGHT - DOT_INSET,
                DOT_RADIUS,
                color,
            )
            .with_anti_alias()
        }));
        col(
            props!(
                width: self.side,
                height: CONTROL_HEIGHT,
                background: if touches.is_held(&self.key) { PRESSED_COLOR } else { TRACK_COLOR },
                border_radius: CONTROL_RADIUS
            ),
            [touchable(&self.key, fill_insets(), draws)],
        )
    }

    /// A finger was lifted from it, which asks for the next page.
    #[must_use]
    pub fn touched(&self, touches: &Touches) -> bool {
        touches.released(&self.key).is_some()
    }

    /// The touches of a finger lifted from it.
    #[must_use]
    pub fn tap(&self) -> Touches {
        Touches::tap(&self.key, middle(self.side, CONTROL_HEIGHT))
    }

    /// The touches of a finger on it.
    #[must_use]
    pub fn hold(&self) -> Touches {
        Touches::hold(&self.key, middle(self.side, CONTROL_HEIGHT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::icons::CHECK;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn a_tap_on_the_page_button_reads_back_as_touched() {
        let button = PageButton {
            key: "page".to_owned(),
            icon: Icon(&CHECK),
            page: 0,
            count: 2,
            side: 64.0,
        };
        assert!(button.touched(&button.tap()));
        assert!(!button.touched(&button.hold()));
    }
}

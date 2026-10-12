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

//! Fleet screen modules, the frame they draw in, plus the shared empty state.

use bmc_wasm_sdk::*;

pub mod bmm101;
pub mod dashboard;
pub mod device_detail;
pub mod fixtures;
pub mod icons;
pub mod model_detail;
pub mod parts;
pub mod table;

use bmm101::Bmm101;
use dashboard::DashboardViewData;
use device_detail::DeviceDetailData;
use model_detail::ModelDetailViewData;
use table::TableViewData;

/// The frame the screens draw: the Deck's full band, or BMM101's own.
#[derive(Clone, Copy, Debug)]
pub enum Layout {
    Deck,
    Bmm101(Bmm101),
}

impl Layout {
    /// Anything no wider than BMM101 takes its frame, fitted to the viewport;
    /// the manifest admits only that range besides the Deck's full band.
    #[must_use]
    pub fn of(width: u32, height: u32) -> Self {
        if width <= Bmm101::DESIGN_WIDTH {
            Self::Bmm101(Bmm101::fit(width, height))
        } else {
            Self::Deck
        }
    }

    #[must_use]
    pub fn dashboard(self, data: &DashboardViewData) -> Node {
        match self {
            Self::Deck => dashboard::dashboard_view(data),
            Self::Bmm101(m) => bmm101::dashboard::dashboard_view(m, data),
        }
    }

    #[must_use]
    pub fn table(self, data: &TableViewData) -> Node {
        match self {
            Self::Deck => table::table_view(data),
            Self::Bmm101(m) => bmm101::table::table_view(m, data),
        }
    }

    #[must_use]
    pub fn model_detail(self, data: &ModelDetailViewData) -> Node {
        match self {
            Self::Deck => model_detail::model_detail_view(data),
            Self::Bmm101(m) => bmm101::model_detail::model_detail_view(m, data),
        }
    }

    #[must_use]
    pub fn device_detail(self, data: &DeviceDetailData) -> Node {
        match self {
            Self::Deck => device_detail::device_detail_view(data),
            Self::Bmm101(m) => bmm101::device_detail::device_detail_view(m, data),
        }
    }

    #[must_use]
    pub fn searching(self) -> Node {
        match self {
            Self::Deck => searching(),
            Self::Bmm101(m) => bmm101::searching(m),
        }
    }
}

/// What the empty fleet asks of the user, on every device.
const SEARCHING_HINT: &str =
    "Make sure miners are on this network and their accounts are picked in the widget settings";

/// Fleet-empty state: discovery is still running, or every device was filtered
/// out by the operator's model lists / disabled families. The indeterminate
/// bar animates host-side, so a stalled discovery still reads as live.
#[must_use]
pub fn searching() -> Node {
    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [col(
                props!(gap: 20.0, cross_align: CrossAlign::Center),
                [
                    text(
                        "Searching for miners\u{2026}",
                        style!(size: 28, color: WHITE),
                    ),
                    text(SEARCHING_HINT, style!(size: 16, color: GRAY_60)),
                    col(
                        props!(width: 260.0),
                        [progress_bar!(
                            ProgressMode::Indeterminate,
                            track_h: 4.0, active: true, fill_color: WHITE,
                            track_color: TRANSPARENT, bg_color: TRANSPARENT,
                        )],
                    ),
                ],
            )],
        )],
    )
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::wasm_test;

    use super::*;

    fn bmm101_scale(width: u32, height: u32) -> f32 {
        match Layout::of(width, height) {
            Layout::Bmm101(m) => m.scale(),
            Layout::Deck => panic!("BUG: {width}x{height} took the Deck frame"),
        }
    }

    #[wasm_test]
    fn the_deck_full_band_keeps_the_deck_frame() {
        assert!(matches!(Layout::of(1_280, 480), Layout::Deck));
    }

    #[wasm_test]
    fn bmm101_draws_its_frame_unscaled() {
        assert!((bmm101_scale(480, 320) - 1.0).abs() < f32::EPSILON);
    }

    #[wasm_test]
    fn bmm100_shrinks_the_bmm101_frame_by_its_tighter_side() {
        let scale = bmm101_scale(320, 240);
        assert!(
            (scale - 2.0 / 3.0).abs() < 1e-6,
            "320 / 480 is tighter than 240 / 320, got {scale}"
        );
    }
}

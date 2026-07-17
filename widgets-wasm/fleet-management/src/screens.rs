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

//! Fleet screen modules plus the shared empty state.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

pub mod dashboard;
pub mod device_detail;
pub mod fixtures;
pub mod icons;
pub mod model_detail;
pub mod no_credentials;
pub mod parts;
pub mod table;

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
                    text(
                        "Make sure miners are on this network and the Deck has their credentials",
                        style!(size: 16, color: GRAY_60),
                    ),
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

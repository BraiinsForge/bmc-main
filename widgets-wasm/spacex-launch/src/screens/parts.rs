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

//! Reusable layout pieces: the label/value tables and the rocket panel.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::model::LaunchData;

const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
const FALCON_HEAVY: Bitmap = include_bitmap!("assets/falcon-heavy.png");
const UNKNOWN_ROCKET: Bitmap = include_bitmap!("assets/unknown.png");

/// How a table row spends its space on a label and its value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ValueLayout {
    /// Label left, value right on one line. Needs width to hold both.
    Inline,
    /// Value under its label. Costs a line per row and gives the value the
    /// row's whole width, for a viewport with height to spare and none to
    /// waste sideways.
    Stacked,
}

/// Single table row: gray label, bold value.
fn table_row(label: &str, value: &str, font_size: u32, layout: ValueLayout) -> Node {
    let label = text(
        label,
        style!(size: font_size, color: GRAY_30, line_height: 1.2),
    );
    let value = text(
        value,
        style!(size: font_size, weight: FontWeight::BOLD, line_height: 1.2),
    );
    match layout {
        // The gap is what keeps the two apart once the value grows enough to
        // wrap: the spacer collapses to nothing at that point, and without it
        // the label and the value touch.
        ValueLayout::Inline => row(props!(gap: 8.0), [label, spacer(1.0), value]),
        ValueLayout::Stacked => col(props!(gap: 2.0), [label, value]),
    }
}

/// Thin horizontal separator line.
fn divider() -> Node {
    col(props!(height: 1.0, background: GRAY_90), [])
}

/// Left table: Scheduled, Status, Rocket, Place.
pub(super) fn launch_info_table(
    font_size: u32,
    gap: f32,
    data: &LaunchData,
    countdown: &str,
    status: &str,
    layout: ValueLayout,
) -> Node {
    col(
        props!(gap: gap, flex: 1.0),
        [
            table_row("Scheduled", countdown, font_size, layout),
            divider(),
            table_row("Status", status, font_size, layout),
            divider(),
            table_row("Rocket", &data.rocket, font_size, layout),
            divider(),
            table_row("Place", &data.place, font_size, layout),
        ],
    )
}

/// Right table: Landing, Booster, Payload, Spacecraft.
pub(super) fn detail_table(
    font_size: u32,
    gap: f32,
    data: &LaunchData,
    layout: ValueLayout,
) -> Node {
    col(
        props!(gap: gap, flex: 1.0),
        [
            table_row("Landing", &data.landing, font_size, layout),
            divider(),
            table_row("Booster", &data.booster, font_size, layout),
            divider(),
            table_row("Payload", &data.payload, font_size, layout),
            divider(),
            table_row("Spacecraft", &data.spacecraft, font_size, layout),
        ],
    )
}

/// Rocket image panel (right side, full-height canvas with bitmap).
pub(super) fn rocket_panel(rocket_name: &str, h: f32) -> Node {
    let bmp = rocket_bitmap(rocket_name);
    canvas(
        props!(width: 320.0, height: h),
        [Draw::bitmap(0.0, 0.0, 320.0, h, bmp)],
    )
}

fn rocket_bitmap(name: &str) -> &'static Bitmap {
    let lower = name.as_bytes();
    let has_falcon = name.contains("Falcon") || name.contains("falcon");
    if has_falcon && (contains_bytes(lower, b"heavy") || contains_bytes(lower, b"Heavy")) {
        &FALCON_HEAVY
    } else if has_falcon && (contains_bytes(lower, b"9") || contains_bytes(lower, b"nine")) {
        &FALCON_9
    } else {
        &UNKNOWN_ROCKET
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

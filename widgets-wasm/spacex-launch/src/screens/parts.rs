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
use std::time::Duration;

use crate::model::LaunchData;

const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
const FALCON_HEAVY: Bitmap = include_bitmap!("assets/falcon-heavy.png");
const UNKNOWN_ROCKET: Bitmap = include_bitmap!("assets/unknown.png");

/// The brand, joined so it never breaks across a line.
pub(super) const BRAND: &str = "Space\u{a0}X";

/// How a table sets its rows: the type of the label and of the value.
#[derive(Clone, Copy)]
pub(super) struct RowStyle {
    pub(super) size: u32,
    pub(super) line_height: f32,
    pub(super) label_color: Color,
    pub(super) value_color: Color,
    pub(super) value_weight: FontWeight,
    /// Where a value's lines sit once it wraps.
    pub(super) value_align: TextAlign,
}

impl RowStyle {
    /// The Deck tables: gray label, bold value.
    const fn deck(size: u32) -> Self {
        Self {
            size,
            line_height: 1.2,
            label_color: GRAY_30,
            value_color: GRAY_10,
            value_weight: FontWeight::BOLD,
            value_align: TextAlign::Left,
        }
    }
}

/// From here out the countdown drops its seconds,
/// so three digits of days still fit the narrowest table.
const SECONDS_UNDER: Duration = Duration::from_hours(100 * 24);

/// The countdown to the net time, and the status beside it.
/// Computed per render, so the timer keeps ticking between nexus refreshes;
/// once the net time passes, the status reads `Launched`.
pub(super) fn countdown(data: &LaunchData, now_secs: i64) -> (String, &str) {
    let remaining = data.launch_unix - now_secs;
    let status = if remaining > 0 {
        data.status.as_str()
    } else {
        "Launched"
    };
    let with_seconds = u64::try_from(remaining).is_ok_and(|secs| secs < SECONDS_UNDER.as_secs());
    // One line: a countdown broken between its units misreads.
    let countdown = typography::unbroken(format_duration(remaining, with_seconds));
    (countdown, status)
}

fn table_row(label: &str, value: &str, style: RowStyle) -> Node {
    let label = text(
        label,
        style!(size: style.size, color: style.label_color, line_height: style.line_height),
    );
    let value = text(
        value,
        style!(
            size: style.size,
            color: style.value_color,
            weight: style.value_weight,
            align: style.value_align,
            line_height: style.line_height,
        ),
    );
    // The gap is what keeps the two apart once the value grows enough to
    // wrap: the spacer collapses to nothing at that point, and without it
    // the label and the value touch.
    row(props!(gap: 8.0), [label, spacer(1.0), value])
}

pub(super) const DIVIDER_THICKNESS: f32 = 1.0;

/// Thin horizontal separator line.
pub(super) fn divider() -> Node {
    col(props!(height: DIVIDER_THICKNESS, background: GRAY_90), [])
}

/// The left table's rows, ruled apart: Scheduled, Status, Rocket, Place.
pub(super) fn launch_info_rows(
    data: &LaunchData,
    countdown: &str,
    status: &str,
    style: RowStyle,
) -> [Node; 7] {
    [
        table_row("Scheduled", countdown, style),
        divider(),
        table_row("Status", status, style),
        divider(),
        table_row("Rocket", &data.rocket, style),
        divider(),
        table_row("Place", &data.place, style),
    ]
}

/// The right table's rows, ruled apart: Landing, Booster, Payload, Spacecraft.
pub(super) fn detail_rows(data: &LaunchData, style: RowStyle) -> [Node; 7] {
    [
        table_row("Landing", &data.landing, style),
        divider(),
        table_row("Booster", &data.booster, style),
        divider(),
        table_row("Payload", &data.payload, style),
        divider(),
        table_row("Spacecraft", &data.spacecraft, style),
    ]
}

/// Left table: Scheduled, Status, Rocket, Place.
pub(super) fn launch_info_table(
    font_size: u32,
    gap: f32,
    data: &LaunchData,
    countdown: &str,
    status: &str,
) -> Node {
    col(
        props!(gap: gap, flex: 1.0),
        launch_info_rows(data, countdown, status, RowStyle::deck(font_size)),
    )
}

/// Right table: Landing, Booster, Payload, Spacecraft.
pub(super) fn detail_table(font_size: u32, gap: f32, data: &LaunchData) -> Node {
    col(
        props!(gap: gap, flex: 1.0),
        detail_rows(data, RowStyle::deck(font_size)),
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

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

//! Header + data-table panels and the per-size layouts.
//!
//! ```text
//!   full (1280×480)            large (638×480)      medium / small
//! ┌──────────────┬───────┐   ┌───────────────┐    ┌───────────────┐
//! │ ISS Position │       │   │ ISS Position  │    │ ISS Position  │
//! │ Orbit  92min │       │   │ Orbit  92min  │    │ Orbit  92min  │
//! │ Alt    420km │ globe │   │ Alt    420km  │    │ Alt    420km  │
//! │ Vel    7 km/s│       │   │ Vis    Sunlit │    │ Vis    Sunlit │
//! │ Over   …     │       │   │ Vel    7 km/s │    │ Vel    7 km/s │
//! └──────────────┴───────┘   │ Over   …      │    │ Over   …      │
//!  Vis row dropped on full:  └───────────────┘    └───────────────┘
//!  the globe terminator shows sunlit vs shadow.
//! ```

use bmc_wasm_sdk::types::Speed;
#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use super::{TITLE, globe};
use crate::model::{IssData, Visibility};
use crate::orbit::ORBIT_PERIOD_MIN;

/// Full (1280×480): header + table on the left, globe on the right.
/// The globe's terminator shows sunlit-vs-shadow, so the table drops
/// the Visibility row.
#[must_use]
pub fn full(data: &IssData, delta_ms: u32) -> Node {
    row(
        props!(background: BLACK),
        [
            col(
                props!(padding: 32.0, flex: 1.0),
                [
                    text(TITLE, style!(size: 24, weight: FontWeight::BOLD)),
                    data_table(24, data, false, true),
                ],
            ),
            globe::map_panel(data, delta_ms),
        ],
    )
}

/// Large (638×480): header + full table, no globe.
#[must_use]
pub fn large(data: &IssData) -> Node {
    col(
        props!(padding: 24.0, background: BLACK),
        [
            text(TITLE, style!(size: 22, weight: FontWeight::BOLD)),
            data_table(18, data, true, true),
        ],
    )
}

/// Medium (638×238): header + table, velocity in km/s only (width).
#[must_use]
pub fn medium(data: &IssData) -> Node {
    col(
        props!(padding: 24.0, background: BLACK),
        [
            text(TITLE, style!(size: 20, weight: FontWeight::BOLD)),
            data_table(20, data, true, false),
        ],
    )
}

/// Small (317×238): header + table, velocity in km/s only (width).
#[must_use]
pub fn small(data: &IssData) -> Node {
    col(
        props!(padding: 16.0, background: BLACK),
        [
            text(TITLE, style!(size: 18, weight: FontWeight::BOLD)),
            data_table(16, data, true, false),
        ],
    )
}

/// Data table shared by every size. `show_visibility` adds the Visibility row
/// (dropped on full, where the globe terminator shows it); `velocity_dual` adds
/// the km/h parenthetical (dropped on the narrow variants for width).
fn data_table(font_size: u32, data: &IssData, show_visibility: bool, velocity_dual: bool) -> Node {
    let mut children: Vec<Node> = Vec::new();
    children.extend(table_entry(
        "Orbit period",
        &fmt!("{} min", ORBIT_PERIOD_MIN),
        font_size,
    ));
    children.extend(table_entry("Altitude", &data.altitude.format(0), font_size));
    if show_visibility {
        children.extend(table_entry(
            "Visibility",
            format_visibility(data.visibility),
            font_size,
        ));
    }
    children.extend(table_entry(
        "Velocity",
        &format_velocity(data.velocity, velocity_dual),
        font_size,
    ));
    children.extend(table_entry_last(
        "Over",
        &format_coords(data.latitude, data.longitude),
        font_size,
    ));
    col(props!(flex: 1.0), children)
}

/// Single table row: gray label left, bold value right.
fn table_row(label: &str, value: &str, font_size: u32) -> Node {
    row(
        props!(),
        [
            text(
                label,
                style!(size: font_size, color: GRAY_30, line_height: 1.2),
            ),
            spacer(1.0),
            text(
                value,
                style!(size: font_size, weight: FontWeight::BOLD, line_height: 1.2),
            ),
        ],
    )
}

/// Thin horizontal separator line.
fn divider() -> Node {
    col(props!(height: 1.0, background: GRAY_90), [])
}

/// A table row group: spacer, row, spacer, divider.
/// The two spacers center the row text between adjacent dividers.
fn table_entry(label: &str, value: &str, font_size: u32) -> [Node; 4] {
    [
        spacer(1.0),
        table_row(label, value, font_size),
        spacer(1.0),
        divider(),
    ]
}

/// Last table row group (no trailing divider).
fn table_entry_last(label: &str, value: &str, font_size: u32) -> [Node; 3] {
    [spacer(1.0), table_row(label, value, font_size), spacer(1.0)]
}

/// Velocity as `7.66 km/s` (`dual` off) or `7.66 km/s (27 571 km/h)` (`dual` on).
/// km/s is the universal orbital-velocity convention shown to every user;
/// only the parenthetical localises (km/h, or mph when imperial) via the host.
fn format_velocity(velocity: Speed, dual: bool) -> String {
    let kms = format_number!(velocity.as_kilometers_per_second(), 2);
    if dual {
        fmt!("{} km/s ({})", kms, velocity.format(0))
    } else {
        fmt!("{} km/s", kms)
    }
}

/// Visibility row value: whether the ISS is currently sunlit or in shadow.
fn format_visibility(vis: Visibility) -> &'static str {
    match vis {
        Visibility::Daylight => "Sunlit",
        Visibility::Eclipsed => "Eclipsed",
    }
}

/// Format coordinates as "X.X°N, X.X°E" using host-side number formatting.
fn format_coords(lat: f64, lon: f64) -> String {
    let lat_dir = if lat >= 0.0 { "N" } else { "S" };
    let lon_dir = if lon >= 0.0 { "E" } else { "W" };
    let lat_str = format_number!(lat.abs(), 1);
    let lon_str = format_number!(lon.abs(), 1);
    fmt!(
        "{}\u{00b0}{}, {}\u{00b0}{}",
        lat_str,
        lat_dir,
        lon_str,
        lon_dir
    )
}

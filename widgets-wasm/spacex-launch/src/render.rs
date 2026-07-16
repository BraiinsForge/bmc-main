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

//! View tree for the SpaceX launch widget: size dispatch, the launch panels,
//! and the loading/error states.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::model::LaunchData;

const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
const FALCON_HEAVY: Bitmap = include_bitmap!("assets/falcon-heavy.png");
const UNKNOWN_ROCKET: Bitmap = include_bitmap!("assets/unknown.png");

/// Dispatch the loaded view by size. The countdown is computed here from the
/// device clock so the timer keeps ticking between nexus refreshes; once the
/// net time passes, the status reads `Launched`.
#[must_use]
pub fn current_view(data: &LaunchData, size: WidgetSize) -> Node {
    let now = SystemTime::now();
    let remaining = data.launch_unix - now.unix_secs;
    let countdown = format_duration(remaining, true);
    let status = if remaining > 0 {
        data.status.as_str()
    } else {
        "Launched"
    };
    match size.variant {
        SizeVariant::Full => render_full(size.height, data, &countdown, status),
        SizeVariant::Large => render_large(data, &countdown, status),
        SizeVariant::Medium => render_medium(data, &countdown, status),
        SizeVariant::Small => render_small(data, &countdown, status),
    }
}

/// Centered loading message.
#[must_use]
pub fn loading_view() -> Node {
    col(
        props!(padding: 32.0, background: BLACK),
        [text("Loading\u{2026}", style!(size: 24, color: GRAY_30))],
    )
}

/// Plain "no upcoming launches" message (valid empty reply, not an error).
#[must_use]
pub fn empty_view() -> Node {
    col(
        props!(padding: 32.0, gap: 16.0, background: BLACK),
        [
            row(
                props!(gap: 8.0),
                [
                    text("Space X", style!(size: 24, color: GRAY_30)),
                    text("Next Launch", style!(size: 24, weight: FontWeight::BOLD)),
                ],
            ),
            text("No upcoming launches", style!(size: 24, color: GRAY_30)),
        ],
    )
}

/// Header plus an error banner with the failure detail.
#[must_use]
pub fn error_view(detail: &str) -> Node {
    col(
        props!(padding: 32.0, gap: 16.0, background: BLACK),
        [
            row(
                props!(gap: 8.0),
                [
                    text("Space X", style!(size: 24, color: GRAY_30)),
                    text("Next Launch", style!(size: 24, weight: FontWeight::BOLD)),
                ],
            ),
            notification(
                NotificationKind::Error,
                "Failed to load launch data",
                detail,
            ),
        ],
    )
}

// ============================================================================
// Reusable layout pieces
// ============================================================================

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

/// Left table: Scheduled, Status, Rocket, Place.
fn launch_info_table(
    font_size: u32,
    gap: f32,
    data: &LaunchData,
    countdown: &str,
    status: &str,
) -> Node {
    col(
        props!(gap: gap, flex: 1.0),
        [
            table_row("Scheduled", countdown, font_size),
            divider(),
            table_row("Status", status, font_size),
            divider(),
            table_row("Rocket", &data.rocket, font_size),
            divider(),
            table_row("Place", &data.place, font_size),
        ],
    )
}

/// Right table: Landing, Booster, Payload, Spacecraft.
fn detail_table(font_size: u32, gap: f32, data: &LaunchData) -> Node {
    col(
        props!(gap: gap, flex: 1.0),
        [
            table_row("Landing", &data.landing, font_size),
            divider(),
            table_row("Booster", &data.booster, font_size),
            divider(),
            table_row("Payload", &data.payload, font_size),
            divider(),
            table_row("Spacecraft", &data.spacecraft, font_size),
        ],
    )
}

/// Rocket image panel (right side, full-height canvas with bitmap).
fn rocket_panel(rocket_name: &str, h: f32) -> Node {
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

// ============================================================================
// Layout variants
// ============================================================================

/// Full (1280×480): header + mission + two tables + rocket panel.
fn render_full(height: u32, data: &LaunchData, countdown: &str, status: &str) -> Node {
    row(
        props!(background: BLACK),
        [
            col(
                props!(padding: 32.0, flex: 1.0, gap: 12.0),
                [
                    // Header
                    row(
                        props!(gap: 8.0),
                        [
                            text("Space X", style!(size: 24, color: GRAY_30)),
                            text("Next Launch", style!(size: 24, weight: FontWeight::BOLD)),
                        ],
                    ),
                    // Mission title
                    text(
                        &data.mission_name,
                        style!(size: 32, weight: FontWeight::BOLD),
                    ),
                    text("Mission name", style!(size: 24, color: GRAY_30)),
                    // Distribute space around tables
                    spacer(1.0),
                    // Two data tables side by side
                    row(
                        props!(gap: 40.0),
                        [
                            launch_info_table(24, 10.0, data, countdown, status),
                            detail_table(24, 10.0, data),
                        ],
                    ),
                    spacer(0.3),
                ],
            ),
            rocket_panel(&data.rocket, height as f32),
        ],
    )
}

/// Large (638×480): header + mission + two tables (stacked), no rocket.
fn render_large(data: &LaunchData, countdown: &str, status: &str) -> Node {
    col(
        props!(padding: 24.0, gap: 8.0, background: BLACK),
        [
            // Header
            row(
                props!(gap: 8.0),
                [
                    text("Space X", style!(size: 22, color: GRAY_30)),
                    text("Next Launch", style!(size: 22, weight: FontWeight::BOLD)),
                ],
            ),
            // Mission title
            col(
                props!(gap: 4.0),
                [
                    text(
                        &data.mission_name,
                        style!(size: 28, weight: FontWeight::BOLD),
                    ),
                    text("Mission name", style!(size: 22, color: GRAY_30)),
                ],
            ),
            spacer(1.0),
            col(
                props!(gap: 32.0),
                [
                    launch_info_table(18, 6.0, data, countdown, status),
                    detail_table(18, 6.0, data),
                ],
            ),
        ],
    )
}

/// Medium (638×238): mission in header, two tables side by side.
fn render_medium(data: &LaunchData, countdown: &str, status: &str) -> Node {
    col(
        props!(padding: 24.0, gap: 8.0, background: BLACK),
        [
            row(
                props!(gap: 8.0),
                [
                    text("Space X", style!(size: 20, color: GRAY_30)),
                    text(
                        &data.mission_name,
                        style!(size: 20, weight: FontWeight::BOLD),
                    ),
                ],
            ),
            spacer(1.0),
            // Smaller font keeps a long detail value on one line at 638×238.
            row(
                props!(gap: 20.0),
                [
                    launch_info_table(16, 6.0, data, countdown, status),
                    detail_table(16, 6.0, data),
                ],
            ),
        ],
    )
}

/// Small (317×238): mission as title, single table.
fn render_small(data: &LaunchData, countdown: &str, status: &str) -> Node {
    col(
        props!(padding: 24.0, gap: 8.0, background: BLACK),
        [
            text(
                &data.mission_name,
                style!(size: 20, weight: FontWeight::BOLD),
            ),
            spacer(1.0),
            launch_info_table(20, 8.0, data, countdown, status),
        ],
    )
}

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

//! View tree for the ISS widget: size dispatch plus loading/error states.

pub mod globe;
pub mod panels;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::model::IssData;

pub(crate) const TITLE: &str = "ISS Position";

/// Dispatch the loaded view by size. `delta_ms` feeds the globe's smoothing on
/// the full variant; the smaller variants ignore it.
#[must_use]
pub fn current_view(data: &IssData, size: WidgetSize, delta_ms: u32) -> Node {
    match size.variant {
        // The globe needs a TLE to draw the orbital track and propagate the
        // live subpoint; without one, fall back to the table-only large view
        // rather than show a bare, drifting globe.
        SizeVariant::Full if data.tle.is_some() => panels::full(data, delta_ms),
        SizeVariant::Full | SizeVariant::Large => panels::large(data),
        SizeVariant::Medium => panels::medium(data),
        SizeVariant::Small => panels::small(data),
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

/// Title plus an error banner with the failure detail.
#[must_use]
pub fn error_view(detail: &str) -> Node {
    col(
        props!(padding: 32.0, gap: 16.0, background: BLACK),
        [
            text(TITLE, style!(size: 24, weight: FontWeight::BOLD)),
            notification(NotificationKind::Error, "Failed to load data", detail),
        ],
    )
}

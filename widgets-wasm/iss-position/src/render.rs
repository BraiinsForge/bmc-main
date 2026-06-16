// Copyright (C) 2026  Braiins Systems s.r.o.

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

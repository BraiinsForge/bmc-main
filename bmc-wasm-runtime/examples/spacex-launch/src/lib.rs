// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(clippy::cast_precision_loss)]

//! SpaceX Launch Widget — WASM runtime demo (BDK-285).
//!
//! Fetches live launch data from the Launch Library 2 API (thespacedevs.com)
//! and displays a countdown timer with mission details.

use std::cell::{Cell, RefCell};

#[allow(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
const FALCON_HEAVY: Bitmap = include_bitmap!("assets/falcon-heavy.png");
const UNKNOWN_ROCKET: Bitmap = include_bitmap!("assets/unknown.png");

#[rustfmt::skip]
const API_URL: &str = "https://ll.thespacedevs.com/2.3.0/launches/upcoming/?search=spacex&limit=1&status__ids=1&mode=detailed";

fn api_auth() -> Option<String> {
    kv::get_string("ll2_api_token")
        .map(|token| fmt!("Authorization: Token {}", token))
}

/// Refresh interval: 5 minutes.
const REFRESH_MS: u32 = 300_000;
/// Retry interval on error: 30 seconds.
const RETRY_MS: u32 = 30_000;

enum WidgetState {
    Loading,
    Loaded(LaunchData),
    Error(String),
}

thread_local! {
    static SIZE: Cell<WidgetSize> = const { Cell::new(WidgetSize {
        variant: SizeVariant::Full,
        width: 1_280,
        height: 480,
    }) };
    static STATE: RefCell<WidgetState> = const { RefCell::new(WidgetState::Loading) };
}

struct LaunchData {
    mission_name: String,
    launch_unix: i64,
    status: String,
    rocket: String,
    place: String,
    landing: String,
    booster: String,
    payload: String,
    spacecraft: String,
}

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    SIZE.set(WidgetSize::from_dimensions(width, height));
    fetch(API_URL, api_auth().as_deref(), on_launch_data);
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let size = SIZE.get();

    let root = STATE.with(|s| {
        let borrow = s.borrow();
        match &*borrow {
            WidgetState::Loaded(data) => {
                let now = SystemTime::now();
                let remaining = data.launch_unix - now.unix_secs;
                let countdown = format_duration(remaining, true);
                let status = if remaining > 0 {
                    &data.status
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
            WidgetState::Error(msg) => col(
                props!(padding: 32.0, gap: 16.0, background: BLACK),
                [
                    row(
                        props!(gap: 8.0),
                        [
                            text("Space X", style!(size: 24, color: GRAY_30)),
                            text("Next Launch", style!(size: 24, weight: 600)),
                        ],
                    ),
                    notification(NotificationKind::Error, "Failed to load launch data", msg),
                ],
            ),
            WidgetState::Loading => col(
                props!(padding: 32.0, background: BLACK),
                [text("Loading\u{2026}", style!(size: 24, color: GRAY_30))],
            ),
        }
    });

    let _ = render_ui(size.width, size.height, root);
    request_frame_after(1_000);
}

// ============================================================================
// Data fetching
// ============================================================================

fn on_launch_data(response: &FetchResponse) {
    if !response.ok() {
        let msg = if response.status == 0 {
            "Network error".into()
        } else {
            fmt!("API request failed ({})", response.status)
        };
        log_error!("launch data fetch failed: {}", msg);
        STATE.with(|s| *s.borrow_mut() = WidgetState::Error(msg));
        request_frame();
        fetch_after(RETRY_MS, API_URL, api_auth().as_deref(), on_launch_data);
        return;
    }

    let json = response.json();

    let mission_name = json
        .str("/results/0/mission/name")
        .or_else(|| json.str("/results/0/name"))
        .unwrap_or_else(|| "Unknown Mission".into());

    // Parse launch date from ISO 8601 string → unix timestamp
    let net_str = json.str("/results/0/net").unwrap_or_default();
    let launch_unix = parse_date(&net_str).unwrap_or(0);

    let status = json
        .str("/results/0/status/name")
        .unwrap_or_else(|| "TBD".into());

    let rocket = json
        .str("/results/0/rocket/configuration/full_name")
        .or_else(|| json.str("/results/0/rocket/configuration/name"))
        .unwrap_or_else(|| "Unknown".into());

    // Location + pad
    let location = json
        .str("/results/0/pad/location/name")
        .unwrap_or_else(|| "Unknown".into());
    let pad = json.str("/results/0/pad/name").unwrap_or_default();
    let place = abbreviate_place(&location, &pad);

    // Landing
    let landing = match json.bool("/results/0/rocket/launcher_stage/0/landing/attempt") {
        Some(false) => "No attempt".into(),
        Some(true) => json
            .str("/results/0/rocket/launcher_stage/0/landing/type/abbrev")
            .unwrap_or_else(|| "Unknown".into()),
        None => "Not confirmed".into(),
    };

    // Booster flights
    let booster = json
        .i64("/results/0/rocket/launcher_stage/0/launcher_flight_number")
        .map_or_else(|| "N/A".into(), format_booster);

    let payload = json
        .str("/results/0/mission/type")
        .unwrap_or_else(|| "N/A".into());

    let spacecraft = json
        .str("/results/0/rocket/spacecraft_stage/0/spacecraft/name")
        .unwrap_or_else(|| "N/A".into());

    STATE.with(|s| {
        *s.borrow_mut() = WidgetState::Loaded(LaunchData {
            mission_name,
            launch_unix,
            status,
            rocket,
            place,
            landing,
            booster,
            payload,
            spacecraft,
        });
    });

    request_frame();
    fetch_after(REFRESH_MS, API_URL, api_auth().as_deref(), on_launch_data);
}

// ============================================================================
// Data formatting helpers
// ============================================================================

fn abbreviate_place(location: &str, pad: &str) -> String {
    let loc = if location.contains("Cape Canaveral") {
        "CCSFS"
    } else if location.contains("Kennedy") {
        "KSC"
    } else if location.contains("Vandenberg") {
        "VSFB"
    } else if location.contains("Starbase") || location.contains("SpaceX") {
        "Starbase"
    } else {
        location
    };
    if pad.is_empty() {
        loc.into()
    } else {
        // Shorten pad name: "Space Launch Complex 40" → "SLC-40"
        let short_pad = pad
            .replace("Space Launch Complex ", "SLC-")
            .replace("Launch Complex ", "LC-")
            .replace("Orbital Launch Mount ", "OLM-");
        fmt!("{} {}", loc, short_pad)
    }
}

fn format_booster(flights: i64) -> String {
    if flights <= 1 {
        "Flight #1".into()
    } else {
        fmt!("{}\u{00d7} flown", flights)
    }
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
                style!(size: font_size, weight: 600, line_height: 1.2),
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
        [bitmap(0.0, 0.0, 320.0, h, bmp)],
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
                            text("Next Launch", style!(size: 24, weight: 600)),
                        ],
                    ),
                    // Mission title
                    text(&data.mission_name, style!(size: 32, weight: 600)),
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
                    text("Next Launch", style!(size: 22, weight: 600)),
                ],
            ),
            // Mission title
            col(
                props!(gap: 4.0),
                [
                    text(&data.mission_name, style!(size: 28, weight: 600)),
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
                    text(&data.mission_name, style!(size: 20, weight: 600)),
                ],
            ),
            spacer(1.0),
            row(
                props!(gap: 24.0),
                [
                    launch_info_table(20, 8.0, data, countdown, status),
                    detail_table(20, 8.0, data),
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
            text(&data.mission_name, style!(size: 20, weight: 600)),
            spacer(1.0),
            launch_info_table(20, 8.0, data, countdown, status),
        ],
    )
}

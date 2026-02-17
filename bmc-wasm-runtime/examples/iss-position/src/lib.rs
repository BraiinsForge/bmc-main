// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::similar_names
)]

//! ISS Position Widget — WASM runtime (BDK-304).
//!
//! Displays live ISS position data with a map tile, orbital track,
//! and day/night terminator overlay (full size variant).

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;

#[allow(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

// ============================================================================
// Constants
// ============================================================================

/// ISS orbital period in minutes (well-known constant).
const ORBIT_PERIOD_MIN: u32 = 92;
/// Refresh interval: 5 minutes.
const REFRESH_MS: u32 = 300_000;
/// Retry interval on error: 30 seconds.
const RETRY_MS: u32 = 30_000;
/// Map canvas dimensions for the full-size variant.
const MAP_W: f32 = 560.0;
const MAP_H: f32 = 480.0;

const API_URL: &str = "https://api.wheretheiss.at/v1/satellites/25544";
const TLE_URL: &str = "https://api.wheretheiss.at/v1/satellites/25544/tles";

#[rustfmt::skip]
const MAPBOX_TOKEN: &str = "pk.eyJ1IjoiYnJhaWluc2ZvcmdlIiwiYSI6ImNta3lmZmU0aTA1dzkzaHM2NGQ5Nmhhc2sifQ.HTDxIOIJ7g_9NvMLlVOuVQ";

/// ISS marker SVG icon (compile-time embedded).
const ISS_ICON: Icon = include_icon!("assets/icon-iss.svg");

/// Orbit track color: #1243cd at 80% opacity.
const ORBIT_COLOR: u32 = 0x1243_CDCC;
/// Marker fill color: #1243cd opaque.
const MARKER_COLOR: u32 = 0x1243_CDFF;
/// Marker outer glow: #1243cd at 20% opacity.
const MARKER_GLOW: u32 = 0x1243_CD33;
/// Marker outer glow radius (matches reference 80×80 marker SVG).
const MARKER_GLOW_R: f32 = 40.0;
/// Marker solid circle radius.
const MARKER_SOLID_R: f32 = 24.0;
/// Terminator shade: black at 25% opacity.
const TERMINATOR_COLOR: u32 = 0x0000_0040;
/// Marker icon size in pixels.
const MARKER_SIZE: f32 = 56.0;

/// Number of points to compute for the orbit ground track.
const ORBIT_POINTS: usize = 60;

// ============================================================================
// Data model
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Daylight,
    Eclipsed,
}

struct IssData {
    latitude: f64,
    longitude: f64,
    #[allow(dead_code)] // available for future use (display altitude row)
    altitude: f64,
    velocity: f64,
    visibility: Visibility,
    solar_lat: f64,
    solar_lon: f64,
    fetched_at: i64,
}

enum WidgetState {
    Loading,
    Loaded(IssData),
    Error(String),
}

thread_local! {
    static SIZE: Cell<WidgetSize> = const { Cell::new(WidgetSize {
        variant: SizeVariant::Full,
        width: 1_280,
        height: 480,
    }) };
    static STATE: RefCell<WidgetState> = const { RefCell::new(WidgetState::Loading) };
    /// Registered bitmap ID for the Mapbox tile (set asynchronously).
    static MAP_TILE: Cell<Option<u16>> = const { Cell::new(None) };
    /// Cached TLE lines for SGP4 orbital propagation.
    static TLE: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    SIZE.set(WidgetSize::from_dimensions(width, height));
    fetch(API_URL, None, on_position_data);
    fetch(TLE_URL, None, on_tle_data);
}

// ============================================================================
// Data fetching
// ============================================================================

fn on_position_data(response: &FetchResponse) {
    if !response.ok() {
        let msg = if response.status == 0 {
            "Network error".into()
        } else {
            fmt!("API request failed ({})", response.status)
        };
        log_error!("position fetch failed: {}", msg);
        STATE.with(|s| *s.borrow_mut() = WidgetState::Error(msg));
        request_frame();
        fetch_after(RETRY_MS, API_URL, None, on_position_data);
        return;
    }

    let json = response.json();

    let latitude = json.f64("/latitude").unwrap_or(0.0);
    let longitude = json.f64("/longitude").unwrap_or(0.0);
    let altitude = json.f64("/altitude").unwrap_or(0.0);
    let velocity = json.f64("/velocity").unwrap_or(0.0);
    let solar_lat = json.f64("/solar_lat").unwrap_or(0.0);
    let solar_lon = json.f64("/solar_lon").unwrap_or(0.0);

    let visibility = match json.str("/visibility").as_deref() {
        Some("eclipsed") => Visibility::Eclipsed,
        _ => Visibility::Daylight,
    };

    let fetched_at = SystemTime::now().unix_secs;

    // Fetch map tile centered on new position
    let url = map_tile_url(latitude, longitude);
    fetch(&url, None, on_map_tile);

    STATE.with(|s| {
        *s.borrow_mut() = WidgetState::Loaded(IssData {
            latitude,
            longitude,
            altitude,
            velocity,
            visibility,
            solar_lat,
            solar_lon,
            fetched_at,
        });
    });

    request_frame();
    fetch_after(REFRESH_MS, API_URL, None, on_position_data);
    fetch_after(REFRESH_MS, TLE_URL, None, on_tle_data);
}

fn on_map_tile(response: &FetchResponse) {
    if !response.ok() {
        log_warn!("map tile fetch failed (status {})", response.status);
        return;
    }
    let bitmap_id = host::register_bitmap(response.body());
    MAP_TILE.set(Some(bitmap_id));
    request_frame();
}

fn on_tle_data(response: &FetchResponse) {
    if !response.ok() {
        return;
    }
    let json = response.json();
    let l1 = json.str("/line1").unwrap_or_default();
    let l2 = json.str("/line2").unwrap_or_default();
    if !l1.is_empty() && !l2.is_empty() {
        TLE.with(|t| *t.borrow_mut() = Some((l1, l2)));
        request_frame();
    }
}

/// Construct Mapbox Static Images API URL for a plain dark tile.
fn map_tile_url(lat: f64, lon: f64) -> String {
    // Mapbox needs plain decimal coordinates — integer math avoids locale formatting.
    let lat_10 = (lat * 10.0) as i64;
    let lon_10 = (lon * 10.0) as i64;
    let lat_sign = if lat_10 < 0 { "-" } else { "" };
    let lon_sign = if lon_10 < 0 { "-" } else { "" };
    let lat_a = if lat_10 < 0 { -lat_10 } else { lat_10 };
    let lon_a = if lon_10 < 0 { -lon_10 } else { lon_10 };
    fmt!(
        "https://api.mapbox.com/styles/v1/mapbox/dark-v11/static/{}{}.{},{}{}.{},1/560x480@2x?logo=false&attribution=false&access_token={}",
        lon_sign,
        lon_a / 10,
        lon_a % 10,
        lat_sign,
        lat_a / 10,
        lat_a % 10,
        MAPBOX_TOKEN
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let size = SIZE.get();

    let root = STATE.with(|s| {
        let borrow = s.borrow();
        match &*borrow {
            WidgetState::Loaded(data) => {
                let now = SystemTime::now();
                let elapsed = now.unix_secs - data.fetched_at;
                let remaining = i64::from(REFRESH_MS) / 1_000 - elapsed;
                let next_update = format_countdown(remaining);
                match size.variant {
                    SizeVariant::Full => render_full(data, &next_update),
                    SizeVariant::Large => render_large(data, &next_update),
                    SizeVariant::Medium => render_medium(data),
                    SizeVariant::Small => render_small(data),
                }
            }
            WidgetState::Error(msg) => col(
                props!(padding: 32.0, gap: 16.0, background: BLACK),
                [
                    text("ISS Position", style!(size: 24, weight: 600)),
                    notification(NotificationKind::Error, "Failed to load data", msg),
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
// Formatting helpers
// ============================================================================

/// Format coordinates as "X.X°N, X.X°E" using host-side number formatting.
fn format_coords(lat: f64, lon: f64) -> String {
    let lat_abs = if lat < 0.0 { -lat } else { lat };
    let lon_abs = if lon < 0.0 { -lon } else { lon };
    let lat_dir = if lat >= 0.0 { "N" } else { "S" };
    let lon_dir = if lon >= 0.0 { "E" } else { "W" };

    let lat_str = format_number!(lat_abs, 1);
    let lon_str = format_number!(lon_abs, 1);

    fmt!(
        "{}\u{00b0}{}, {}\u{00b0}{}",
        lat_str,
        lat_dir,
        lon_str,
        lon_dir
    )
}

/// Format visibility enum to display string.
fn format_visibility(vis: Visibility) -> &'static str {
    match vis {
        Visibility::Daylight => "In sunlight",
        Visibility::Eclipsed => "In Earth shadow",
    }
}

/// Format countdown as "Xm XXs" for the "Next update" row.
fn format_countdown(remaining_secs: i64) -> String {
    if remaining_secs <= 0 {
        return String::from("now");
    }
    let m = remaining_secs / 60;
    let s = remaining_secs % 60;
    if s < 10 {
        fmt!("{}m 0{}s", m, s)
    } else {
        fmt!("{}m {}s", m, s)
    }
}

// ============================================================================
// Orbital math
// ============================================================================

/// Greenwich Mean Sidereal Time in radians from a Unix timestamp.
fn gmst_radians(unix_secs: f64) -> f64 {
    let jd = unix_secs / 86_400.0 + 2_440_587.5;
    let t = (jd - 2_451_545.0) / 36_525.0;
    // Vallado (4th ed.) GMST formula — result in seconds of sidereal time
    let gmst_sec =
        67_310.548_41 + (876_600.0 * 3_600.0 + 8_640_184.812_866) * t + 0.093_104 * t * t
            - 6.2e-6 * t * t * t;
    // Normalize to [0, 86400) then convert to radians
    let s = ((gmst_sec % 86_400.0) + 86_400.0) % 86_400.0;
    s / 86_400.0 * 2.0 * PI
}

/// Convert ECI (TEME) position to geodetic latitude/longitude (degrees).
fn eci_to_geodetic(pos: &[f64; 3], gmst: f64) -> (f64, f64) {
    let x = pos[0];
    let y = pos[1];
    let z = pos[2];
    let lon_rad = y.atan2(x) - gmst;
    let lat_rad = z.atan2((x * x + y * y).sqrt());
    let lat = lat_rad.to_degrees();
    let lon = ((lon_rad.to_degrees() + 540.0) % 360.0) - 180.0;
    (lat, lon)
}

/// Convert geographic (lat, lon) to pixel position on the map canvas.
///
/// Uses Web Mercator projection matching the Mapbox tile. At zoom 1 the full
/// world is 512 CSS pixels (256 × 2¹). The canvas viewport (w × h) is centered
/// on (center_lat, center_lon). No modulo wrapping — antimeridian splitting in
/// `compute_ground_track` already guarantees no segment crosses ±180°.
fn geo_to_pixel(
    lat: f64,
    lon: f64,
    center_lat: f64,
    center_lon: f64,
    w: f32,
    h: f32,
) -> (f32, f32) {
    // Full world size at zoom 1 in CSS pixels (Mapbox convention)
    let scale = 512.0_f64;
    let world_x = |lon: f64| (lon + 180.0) / 360.0 * scale;
    let world_y = |lat: f64| {
        let r = lat.to_radians();
        (1.0 - (r.tan() + 1.0 / r.cos()).ln() / PI) / 2.0 * scale
    };
    let cx = world_x(center_lon);
    let cy = world_y(center_lat);
    let px = world_x(lon) - cx + f64::from(w) / 2.0;
    let py = world_y(lat) - cy + f64::from(h) / 2.0;
    (px as f32, py as f32)
}

/// Compute ground track as a single continuous polyline from TLE + SGP4.
///
/// Instead of splitting at the antimeridian, we *unwrap* longitude so it
/// increases monotonically.  The caller draws shifted copies (±world width)
/// to handle the map wrapping — giving a perfectly continuous curve like
/// wheretheiss.at.
fn compute_ground_track(
    tle_l1: &str,
    tle_l2: &str,
    center_lat: f64,
    center_lon: f64,
) -> Vec<(f32, f32)> {
    let Ok(elements) = sgp4::Elements::from_tle(None, tle_l1.as_bytes(), tle_l2.as_bytes()) else {
        return Vec::new();
    };

    let Ok(constants) = sgp4::Constants::from_elements(&elements) else {
        return Vec::new();
    };

    let epoch_unix = elements.datetime.and_utc().timestamp() as f64;
    let now_unix = SystemTime::now().unix_secs as f64;
    let minutes_since_epoch = (now_unix - epoch_unix) / 60.0;

    // Anchor: propagate at t=0 to find the SGP4 position at "now", then
    // compute the longitude delta vs the API-reported center_lon.  This
    // corrects for any TLE staleness or GMST precision drift so the track
    // stays centered on the map tile.
    let t0 = sgp4::MinutesSinceEpoch(minutes_since_epoch);
    let anchor = if let Ok(p0) = constants.propagate(t0) {
        let gmst0 = gmst_radians(now_unix);
        let (_, sgp4_lon) = eci_to_geodetic(&p0.position, gmst0);
        let mut d = center_lon - sgp4_lon;
        if d > 180.0 {
            d -= 360.0;
        }
        if d < -180.0 {
            d += 360.0;
        }
        d
    } else {
        0.0
    };

    // Propagate slightly longer than one orbit so the track endpoints extend
    // past the canvas edges (clipped by scissor), matching Mapbox's edge-to-edge
    // overlay rendering.  +20 min ≈ +73° lon ≈ +104px per side at zoom 1.
    let duration_min = f64::from(ORBIT_PERIOD_MIN) + 20.0;
    let half = duration_min / 2.0;
    let interval = duration_min / ORBIT_POINTS as f64;

    // Collect geographic positions and unwrap longitude
    let mut pixels: Vec<(f32, f32)> = Vec::with_capacity(ORBIT_POINTS);
    let mut prev_lon = f64::NAN;

    for i in 0..ORBIT_POINTS {
        let offset = -half + i as f64 * interval;
        let t = sgp4::MinutesSinceEpoch(minutes_since_epoch + offset);
        let Ok(prediction) = constants.propagate(t) else {
            continue;
        };

        let prop_unix = now_unix + offset * 60.0;
        let gmst = gmst_radians(prop_unix);
        let (lat, mut lon) = eci_to_geodetic(&prediction.position, gmst);
        lon += anchor;

        // Unwrap: adjust lon so the delta from previous never jumps > 180°
        if prev_lon.is_finite() {
            let mut d = lon - prev_lon;
            if d > 180.0 {
                d -= 360.0;
            }
            if d < -180.0 {
                d += 360.0;
            }
            lon = prev_lon + d;
        }
        prev_lon = lon;

        pixels.push(geo_to_pixel(lat, lon, center_lat, center_lon, MAP_W, MAP_H));
    }

    // The longitude unwrapping can push points past ±180° (e.g. from -179°
    // to +181°), placing them one full world width (512 px at zoom 1) away
    // from the map center.  Shift the entire polyline by the nearest
    // multiple of the world width so the midpoint sits on the canvas.
    if pixels.len() > 1 {
        let mid_x = pixels[pixels.len() / 2].0;
        let world_px = 512.0_f32;
        let shift = ((MAP_W / 2.0 - mid_x) / world_px).round() * world_px;
        if shift.abs() > 1.0 {
            for (x, _) in &mut pixels {
                *x += shift;
            }
        }
    }

    pixels
}

/// Compute day/night terminator shade polygon points.
///
/// Uses the same linear y-approximation as the reference widget:
/// `y = h/2 - (terminator_lat / 180) * h`. At zoom 1 this is close enough
/// to Mercator and matches the reference visuals exactly.
fn terminator_points(
    solar_lat: f64,
    solar_lon: f64,
    center_lon: f64,
    w: f32,
    h: f32,
) -> Vec<(f32, f32)> {
    let solar_lat_rad = solar_lat.to_radians();
    let night_above = solar_lat < 0.0;
    let steps = 140;
    let mut pts: Vec<(f32, f32)> = Vec::with_capacity(steps + 4);

    if night_above {
        pts.push((0.0, 0.0));
    } else {
        pts.push((0.0, h));
    }

    for i in 0..=steps {
        let frac = i as f64 / steps as f64;
        let xf = frac as f32 * w;
        let lon = center_lon + (frac - 0.5) * 360.0;
        let lon_diff = (lon - solar_lon).to_radians();
        let term_lat = if solar_lat_rad.abs() < 0.002 {
            0.0
        } else {
            (-lon_diff.cos() / solar_lat_rad.tan()).atan().to_degrees()
        };
        let y = h / 2.0 - (term_lat as f32 / 180.0) * h;
        pts.push((xf, y));
    }

    if night_above {
        pts.push((w, 0.0));
    } else {
        pts.push((w, h));
    }

    pts
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

/// 5-row data table for full/large variants.
fn data_table_full(font_size: u32, data: &IssData, next_update: &str) -> Node {
    let mut children: Vec<Node> = Vec::new();
    children.extend(table_entry(
        "Orbit period",
        &fmt!("{} min", ORBIT_PERIOD_MIN),
        font_size,
    ));
    children.extend(table_entry("Next update", next_update, font_size));
    children.extend(table_entry(
        "In sunlight",
        format_visibility(data.visibility),
        font_size,
    ));
    children.extend(table_entry(
        "Velocity",
        &format_speed!(data.velocity, 0),
        font_size,
    ));
    children.extend(table_entry_last(
        "Over",
        &format_coords(data.latitude, data.longitude),
        font_size,
    ));
    col(props!(flex: 1.0), children)
}

/// 3-row data table for medium/small variants.
fn data_table_compact(font_size: u32, data: &IssData) -> Node {
    let mut children: Vec<Node> = Vec::new();
    children.extend(table_entry(
        "Orbit",
        &fmt!("{} min", ORBIT_PERIOD_MIN),
        font_size,
    ));
    children.extend(table_entry(
        "Velocity",
        &format_speed!(data.velocity, 0),
        font_size,
    ));
    children.extend(table_entry_last(
        "Over",
        &format_coords(data.latitude, data.longitude),
        font_size,
    ));
    col(props!(flex: 1.0), children)
}

// ============================================================================
// Map panel (full-size only)
// ============================================================================

/// Render the map canvas with all overlay layers.
fn map_panel(data: &IssData) -> Node {
    let mut draws: Vec<Draw> = Vec::with_capacity(16);

    // Layer 0: Mapbox tile or dark fallback
    if let Some(id) = MAP_TILE.get() {
        draws.push(Draw::Bitmap {
            x: 0.0,
            y: 0.0,
            w: MAP_W,
            h: MAP_H,
            bitmap_id: id,
        });
    } else {
        draws.push(rect(0.0, 0.0, MAP_W, MAP_H, GRAY_100));
    }

    // Layer 1: terminator shade (day/night boundary)
    let shade = terminator_points(data.solar_lat, data.solar_lon, data.longitude, MAP_W, MAP_H);
    draws.push(path!(shade, fill, color: TERMINATOR_COLOR));

    // Layer 2: orbit ground track (smooth polyline from SGP4)
    // The anchor correction in compute_ground_track ensures the track is
    // centered on the API position, so no extra shifting is needed.
    TLE.with(|t| {
        if let Some((l1, l2)) = &*t.borrow() {
            let track = compute_ground_track(l1, l2, data.latitude, data.longitude);
            if track.len() > 1 {
                draws.push(path!(track, stroke: 4.0, color: ORBIT_COLOR, smooth));
            }
        }
    });

    // Layer 3: ISS marker at canvas center
    let cx = MAP_W / 2.0;
    let cy = MAP_H / 2.0;
    draws.push(circle(cx, cy, MARKER_GLOW_R, MARKER_GLOW));
    draws.push(circle(cx, cy, MARKER_SOLID_R, MARKER_COLOR));
    draws.push(icon(
        cx - MARKER_SIZE / 2.0,
        cy - MARKER_SIZE / 2.0,
        MARKER_SIZE,
        MARKER_SIZE,
        &ISS_ICON,
        WHITE,
    ));

    canvas(props!(width: MAP_W, height: MAP_H), draws)
}

// ============================================================================
// Layout variants
// ============================================================================

/// Full (1280×480): header + 5-row table + map canvas.
fn render_full(data: &IssData, next_update: &str) -> Node {
    row(
        props!(background: BLACK),
        [
            col(
                props!(padding: 32.0, flex: 1.0),
                [
                    text("ISS Position", style!(size: 24, weight: 600)),
                    data_table_full(24, data, next_update),
                ],
            ),
            map_panel(data),
        ],
    )
}

/// Large (638×480): header + 5-row table, no map.
fn render_large(data: &IssData, next_update: &str) -> Node {
    col(
        props!(padding: 24.0, background: BLACK),
        [
            text("ISS Position", style!(size: 22, weight: 600)),
            data_table_full(18, data, next_update),
        ],
    )
}

/// Medium (638×238): header + 3-row compact table.
fn render_medium(data: &IssData) -> Node {
    col(
        props!(padding: 24.0, background: BLACK),
        [
            text("ISS Position", style!(size: 20, weight: 600)),
            data_table_compact(20, data),
        ],
    )
}

/// Small (317×238): header + 3-row compact table.
fn render_small(data: &IssData) -> Node {
    col(
        props!(padding: 16.0, background: BLACK),
        [
            text("ISS Position", style!(size: 18, weight: 600)),
            data_table_compact(16, data),
        ],
    )
}

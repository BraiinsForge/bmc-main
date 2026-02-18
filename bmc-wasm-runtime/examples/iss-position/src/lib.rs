// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::similar_names
)]

//! ISS Position Widget — WASM runtime (BDK-304).
//!
//! Displays live ISS position on an equirectangular globe with orbital track,
//! day/night terminator overlay, and data panels (full/large/medium/small).

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

/// Earth texture for globe rendering (Natural Earth dark theme, equirectangular).
const EARTH_TEXTURE: Bitmap = include_bitmap!("textures/natural-earth-dark.jpg");
/// Globe zoom factor (intuitive scale):
/// - `1.0` = default full globe view
/// - `>1.0` zooms in, `<1.0` zooms out
///
/// The shader expects a camera distance (>1.0) in sphere radii, so we remap
/// this value before sending it to the GPU.
const GLOBE_ZOOM: f32 = 1.0;
/// Debug: time speed multiplier for globe rotation (1.0 = real-time, 60.0 = 1 orbit/~1.5 min).
const TIME_SPEED: f64 = 1.0;

/// Remap the user-facing zoom scale to the shader's camera distance.
fn globe_zoom_to_camera(zoom: f32) -> f32 {
    // Keep a sane, intuitive range around the default.
    let zoom = zoom.clamp(0.6, 1.6);
    let camera = 1.8 / zoom;
    camera.clamp(1.2, 2.6)
}

/// Smoothing time constant for globe center
/// - lower = snappier
/// - higher = smoother
const GLOBE_SMOOTH_MS: f64 = 300.0;
const ISS_ICON: Icon = include_icon!("assets/icon-iss.svg");

const MARKET_COLOR: u32 = VIOLET_70;
const ORBIT_COLOR: u32 = color!(MARKET_COLOR, alpha: 0.8);
const MARKER_GLOW_COLOR: u32 = color!(MARKET_COLOR, alpha: 0.2);
const MARKER_GLOW_R: f32 = 40.0;
const MARKER_SOLID_R: f32 = 24.0;
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
    /// Cached TLE lines for SGP4 orbital propagation.
    static TLE: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
    /// Smoothed globe center (lat, lon) in degrees.
    static SMOOTHED_CENTER: RefCell<Option<(f64, f64)>> = const { RefCell::new(None) };
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
                let has_tle = TLE.with(|t| t.borrow().is_some());
                match size.variant {
                    // Show map panel only once TLE is loaded (avoids orbit track
                    // popping in and inaccurate API-fallback positioning).
                    SizeVariant::Full if has_tle => render_full(data, &next_update, _delta_ms),
                    SizeVariant::Full | SizeVariant::Large => render_large(data, &next_update),
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

    // Full variant has the 3D globe — render at ~30fps for smooth SGP4 rotation.
    // Other variants are static text, 1fps is fine.
    let interval = if size.variant == SizeVariant::Full {
        33
    } else {
        1_000
    };
    request_frame_after(interval);
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

/// Propagate the current ISS position from TLE + SGP4 for smooth real-time tracking.
///
/// Falls back to `None` if TLE is unavailable or propagation fails.
fn propagate_current(tle_l1: &str, tle_l2: &str) -> Option<(f64, f64)> {
    let elements = sgp4::Elements::from_tle(None, tle_l1.as_bytes(), tle_l2.as_bytes()).ok()?;
    let constants = sgp4::Constants::from_elements(&elements).ok()?;
    let epoch_unix = elements.datetime.and_utc().timestamp() as f64;
    let now_unix = SystemTime::now().unix_secs as f64;
    // Accelerate time relative to TLE epoch (TIME_SPEED=1.0 → real-time)
    let effective_unix = epoch_unix + (now_unix - epoch_unix) * TIME_SPEED;
    let t = sgp4::MinutesSinceEpoch((effective_unix - epoch_unix) / 60.0);
    let prediction = constants.propagate(t).ok()?;
    let gmst = gmst_radians(effective_unix);
    Some(eci_to_geodetic(&prediction.position, gmst))
}

/// Compute ground track as geographic coordinates from TLE + SGP4.
///
/// Returns `(lat_deg, lon_deg)` pairs for one full orbit centered on "now".
fn compute_ground_track(
    tle_l1: &str,
    tle_l2: &str,
    anchor_center_lon: Option<f64>,
) -> Vec<(f64, f64)> {
    let Ok(elements) = sgp4::Elements::from_tle(None, tle_l1.as_bytes(), tle_l2.as_bytes()) else {
        return Vec::new();
    };

    let Ok(constants) = sgp4::Constants::from_elements(&elements) else {
        return Vec::new();
    };

    let epoch_unix = elements.datetime.and_utc().timestamp() as f64;
    let now_unix = SystemTime::now().unix_secs as f64;
    let effective_unix = epoch_unix + (now_unix - epoch_unix) * TIME_SPEED;
    let minutes_since_epoch = (effective_unix - epoch_unix) / 60.0;

    // Optional anchor correction: align track to a provided center lon
    // (e.g. API-reported position if TLE is stale). Disable for smooth
    // SGP4-only motion to avoid jumps.
    let anchor = if let Some(center_lon) = anchor_center_lon {
        let t0 = sgp4::MinutesSinceEpoch(minutes_since_epoch);
        if let Ok(p0) = constants.propagate(t0) {
            let gmst0 = gmst_radians(effective_unix);
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
        }
    } else {
        0.0
    };

    let duration_min = f64::from(ORBIT_PERIOD_MIN);
    let half = duration_min / 2.0;
    let interval = duration_min / ORBIT_POINTS as f64;

    let mut points: Vec<(f64, f64)> = Vec::with_capacity(ORBIT_POINTS);

    for i in 0..ORBIT_POINTS {
        let offset = -half + i as f64 * interval;
        let t = sgp4::MinutesSinceEpoch(minutes_since_epoch + offset);
        let Ok(prediction) = constants.propagate(t) else {
            continue;
        };

        let prop_unix = effective_unix + offset * 60.0;
        let gmst = gmst_radians(prop_unix);
        let (lat, mut lon) = eci_to_geodetic(&prediction.position, gmst);
        lon += anchor;
        // Normalize to [-180, 180]
        lon = ((lon + 540.0) % 360.0) - 180.0;

        points.push((lat, lon));
    }

    points
}

/// Project geographic orbit points onto the 3D globe view.
///
/// Returns visible polyline segments as canvas pixel coordinates. Points on the
/// far side of the globe break the polyline into separate segments.
fn project_orbit_to_globe(
    points: &[(f64, f64)],
    center_lat: f64,
    center_lon: f64,
    zoom: f64,
    w: f32,
    h: f32,
) -> Vec<Vec<(f32, f32)>> {
    let clat = center_lat.to_radians();
    let clon = center_lon.to_radians();
    let (cos_clat, sin_clat) = (clat.cos(), clat.sin());
    let (cos_clon, sin_clon) = (clon.cos(), clon.sin());
    let aspect = f64::from(w) / f64::from(h);

    let mut segments: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();

    for &(lat_deg, lon_deg) in points {
        let lat = lat_deg.to_radians();
        let lon = lon_deg.to_radians();

        // Geographic → unit sphere
        let px = lat.cos() * lon.sin();
        let py = lat.sin();
        let pz = lat.cos() * lon.cos();

        // Forward rotation: Rx(clat) * Ry(-clon) — inverse of shader's undo
        // Ry(-clon)
        let p1x = px * cos_clon - pz * sin_clon;
        let p1y = py;
        let p1z = px * sin_clon + pz * cos_clon;
        // Rx(clat)
        let vx = p1x;
        let vy = p1y * cos_clat - p1z * sin_clat;
        let vz = p1y * sin_clat + p1z * cos_clat;

        // Back-face cull: generous margin so Catmull-Rom smoothing can't overshoot
        if vz <= 0.40 {
            if current.len() > 1 {
                segments.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            continue;
        }

        // Perspective projection matching the shader's camera
        let scale = zoom / (zoom - vz);
        let uv_x = vx * scale / aspect;
        let uv_y = vy * scale;

        // UV [-1, 1] → canvas pixels (Y flipped for top-down canvas)
        let canvas_x = ((uv_x + 1.0) / 2.0 * f64::from(w)) as f32;
        let canvas_y = ((1.0 - uv_y) / 2.0 * f64::from(h)) as f32;

        current.push((canvas_x, canvas_y));
    }

    if current.len() > 1 {
        segments.push(current);
    }

    segments
}

/// Smoothly approach target lat/lon (degrees) with exponential easing.
struct SmoothedCenter {
    lat: f64,
    lon: f64,
}

fn smooth_globe_center(target_lat: f64, target_lon: f64, delta_ms: u32) -> SmoothedCenter {
    let dt = f64::from(delta_ms).min(1000.0);
    let alpha = 1.0 - (-dt / GLOBE_SMOOTH_MS).exp();

    SMOOTHED_CENTER.with(|c| {
        let mut c = c.borrow_mut();
        let (mut lat, mut lon) = match *c {
            Some((lat, lon)) => (lat, lon),
            None => (target_lat, target_lon),
        };

        lat += (target_lat - lat) * alpha;

        let mut dlon = target_lon - lon;
        if dlon > 180.0 {
            dlon -= 360.0;
        }
        if dlon < -180.0 {
            dlon += 360.0;
        }
        lon += dlon * alpha;

        // Normalize to [-180, 180]
        lon = ((lon + 540.0) % 360.0) - 180.0;

        *c = Some((lat, lon));
        SmoothedCenter { lat, lon }
    })
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

/// Render the 3D globe canvas with orbit track and ISS marker overlays.
fn map_panel(data: &IssData, delta_ms: u32) -> Node {
    let mut draws: Vec<Draw> = Vec::with_capacity(16);
    let globe_zoom = globe_zoom_to_camera(GLOBE_ZOOM);

    // Use SGP4 real-time position for smooth globe rotation between API updates.
    // Falls back to the last API position if TLE is unavailable.
    let mut use_anchor = false;
    let (globe_lat, globe_lon) = TLE.with(|t| {
        t.borrow()
            .as_ref()
            .and_then(|(l1, l2)| propagate_current(l1, l2))
            .unwrap_or_else(|| {
                use_anchor = true;
                (data.latitude, data.longitude)
            })
    });
    let smoothed = smooth_globe_center(globe_lat, globe_lon, delta_ms);

    // Layer 0: 3D sphere — shader handles rotation, light shading, and terminator.
    // Always wrap in .transition() so the host can interpolate sphere params.
    // On the first frame, smoothed center == target so no visible animation fires.
    draws.push(
        sphere!(
            &EARTH_TEXTURE,
            at: (0.0, 0.0, MAP_W, MAP_H),
            center: (smoothed.lat as f32, smoothed.lon as f32),
            zoom: globe_zoom,
            light: (data.solar_lat as f32, data.solar_lon as f32),
            atmosphere
        )
        .transition(250, Easing::EaseOut),
    );

    // Layer 1: orbit ground track projected onto the 3D globe
    TLE.with(|t| {
        if let Some((l1, l2)) = &*t.borrow() {
            let anchor_lon = if use_anchor {
                Some(data.longitude)
            } else {
                None
            };
            let geo_track = compute_ground_track(l1, l2, anchor_lon);
            let segments = project_orbit_to_globe(
                &geo_track,
                smoothed.lat,
                smoothed.lon,
                f64::from(globe_zoom),
                MAP_W,
                MAP_H,
            );
            for seg in segments {
                if seg.len() > 1 {
                    draws.push(path!(seg, stroke: 3.0, color: ORBIT_COLOR, smooth));
                }
            }
        }
    });

    // Layer 2: ISS marker at globe center (the globe rotates to face the ISS position)
    let cx = MAP_W / 2.0;
    let cy = MAP_H / 2.0;
    draws.push(circle(cx, cy, MARKER_GLOW_R, MARKER_GLOW_COLOR));
    draws.push(circle(cx, cy, MARKER_SOLID_R, MARKET_COLOR));
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
fn render_full(data: &IssData, next_update: &str, delta_ms: u32) -> Node {
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
            map_panel(data, delta_ms),
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

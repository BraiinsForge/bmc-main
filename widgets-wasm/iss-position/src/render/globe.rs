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

//! The 3D globe canvas: textured sphere, orbital track, and centered ISS
//! marker. The globe rotates so the live subpoint stays under the marker.

use std::cell::RefCell;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::model::IssData;
use crate::orbit;

/// Map canvas dimensions for the full-size variant.
const MAP_W: f32 = 560.0;
const MAP_H: f32 = 480.0;

/// Earth basemap (equirectangular). Regenerate via the `tools/` texture
/// pipeline and promote the chosen render here as `texture.jpg`.
const EARTH_TEXTURE: Bitmap = include_bitmap!("src/render/texture.jpg");
const ISS_ICON: Svg = include_svg!("assets/icon-iss.svg");

/// Globe zoom (`1.0` = default full-globe view; `>1.0` zooms in).
const GLOBE_ZOOM: f32 = 1.0;
/// Smoothing time constant for the globe center: lower snappier, higher smoother.
const GLOBE_SMOOTH_MS: f64 = 300.0;

const MARKER_COLOR: Color = BLUE_70;
const ORBIT_COLOR: Color = MARKER_COLOR.with_alpha(0.8);
const MARKER_GLOW_COLOR: Color = MARKER_COLOR.with_alpha(0.2);
const MARKER_GLOW_R: f32 = 40.0;
const MARKER_SOLID_R: f32 = 24.0;
const MARKER_SIZE: f32 = 56.0;

/// Cached ground track: its 60 SGP4 propagations are recomputed only when older
/// than [`TRACK_MAX_AGE_SECS`] (the orbit shifts only over minutes), never every frame.
/// Projection onto the moving globe stays per-frame — that's cheap trig.
struct CachedTrack {
    computed_at: f64,
    anchor: Option<f64>,
    geo: Vec<(f64, f64)>,
}

/// Max age of a cached track; over this the "now" point shifts well
/// under one track-point's spacing, so reuse is imperceptible.
const TRACK_MAX_AGE_SECS: f64 = 10.0;

thread_local! {
    /// Smoothed globe center (lat, lon) in degrees, eased toward the live subpoint.
    static SMOOTHED_CENTER: RefCell<Option<(f64, f64)>> = const { RefCell::new(None) };
    static TRACK_CACHE: RefCell<Option<CachedTrack>> = const { RefCell::new(None) };
}

/// Render the globe canvas with the orbital track and centered ISS marker.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "unix-second time math is exact in f64; the f32 canvas-geometry downcasts are intended"
)]
pub fn map_panel(data: &IssData, delta_ms: u32) -> Node {
    let mut draws: Vec<Draw> = Vec::with_capacity(16);
    let globe_zoom = orbit::globe_zoom_to_camera(GLOBE_ZOOM);
    let now_unix = SystemTime::now().unix_secs as f64;

    // Prefer the live SGP4 subpoint so the globe rotates smoothly between
    // refreshes; fall back to the reported position if propagation fails.
    let mut use_anchor = false;
    let (globe_lat, globe_lon) = {
        let _s = profile::span("propagate");
        data.tle
            .as_ref()
            .and_then(|tle| orbit::propagate_at(tle, now_unix))
            .unwrap_or_else(|| {
                use_anchor = true;
                (data.latitude, data.longitude)
            })
    };
    let smoothed = smooth_globe_center(globe_lat, globe_lon, delta_ms);

    // Layer 0: textured sphere — the shader handles rotation, light shading and
    // the terminator. Wrapped in a transition so the host interpolates the
    // sphere params; on the first frame target == smoothed so nothing animates.
    draws.push(
        sphere!(
            &EARTH_TEXTURE,
            at: (0.0, 0.0, MAP_W, MAP_H),
            center: (smoothed.lat as f32, smoothed.lon as f32),
            zoom: globe_zoom,
            light: (data.solar_lat as f32, data.solar_lon as f32),
            atmosphere
        )
        .transition("earth-sphere", 250, Easing::EaseOut),
    );

    // Layer 1: orbital ground track (SGP4 cached; only projection runs per frame).
    if let Some(tle) = &data.tle {
        let anchor = use_anchor.then_some(data.longitude);
        let _s = profile::span("track");
        let segments = TRACK_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let stale = cache.as_ref().is_none_or(|c| {
                c.anchor != anchor || (now_unix - c.computed_at).abs() > TRACK_MAX_AGE_SECS
            });
            if stale {
                *cache = Some(CachedTrack {
                    computed_at: now_unix,
                    anchor,
                    geo: orbit::ground_track(tle, now_unix, anchor),
                });
            }
            let geo = &cache.as_ref().expect("BUG: populated when stale").geo;
            orbit::project_orbit_to_globe(
                geo,
                smoothed.lat,
                smoothed.lon,
                f64::from(globe_zoom),
                MAP_W,
                MAP_H,
            )
        });
        for seg in segments {
            if seg.len() > 1 {
                draws.push(path!(seg, stroke: 3.0, color: ORBIT_COLOR, smooth));
            }
        }
    }

    // Layer 2: ISS marker pinned at globe center (the globe rotates to it).
    let cx = MAP_W / 2.0;
    let cy = MAP_H / 2.0;
    draws.push(Draw::circle(cx, cy, MARKER_GLOW_R, MARKER_GLOW_COLOR));
    draws.push(Draw::circle(cx, cy, MARKER_SOLID_R, MARKER_COLOR));
    draws.push(Draw::svg(
        cx - MARKER_SIZE / 2.0,
        cy - MARKER_SIZE / 2.0,
        MARKER_SIZE,
        MARKER_SIZE,
        &ISS_ICON,
        WHITE,
    ));

    canvas(props!(width: MAP_W, height: MAP_H), draws)
}

/// Smoothly approached globe center in degrees.
struct SmoothedCenter {
    lat: f64,
    lon: f64,
}

/// Ease the globe center toward the target lat/lon with exponential smoothing,
/// taking the shortest path across the antimeridian.
fn smooth_globe_center(target_lat: f64, target_lon: f64, delta_ms: u32) -> SmoothedCenter {
    let dt = f64::from(delta_ms).min(1000.0);
    let alpha = 1.0 - (-dt / GLOBE_SMOOTH_MS).exp();

    SMOOTHED_CENTER.with(|c| {
        let mut c = c.borrow_mut();
        let (mut lat, mut lon) = c.unwrap_or((target_lat, target_lon));

        lat += (target_lat - lat) * alpha;

        let mut dlon = target_lon - lon;
        if dlon > 180.0 {
            dlon -= 360.0;
        }
        if dlon < -180.0 {
            dlon += 360.0;
        }
        lon += dlon * alpha;
        lon = ((lon + 540.0) % 360.0) - 180.0;

        *c = Some((lat, lon));
        SmoothedCenter { lat, lon }
    })
}

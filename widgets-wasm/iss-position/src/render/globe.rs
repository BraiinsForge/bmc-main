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

use crate::model::{IssData, Tle};
use crate::{orbit, orbit_cache};

/// Map canvas dimensions for the full-size variant.
const MAP_W: f32 = 560.0;
const MAP_H: f32 = 480.0;

/// Earth basemap (equirectangular). Regenerate via the `tools/` texture
/// pipeline and promote the chosen render here as `texture.jpg`.
const EARTH_TEXTURE: Bitmap = include_bitmap!("src/render/texture.jpg");
const ISS_ICON: Svg = include_svg!("assets/icon-iss.svg");

/// Globe zoom (`1.0` = default full-globe view; `>1.0` zooms in).
const GLOBE_ZOOM: f32 = 1.0;
const MARKER_COLOR: Color = BLUE_70;
const ORBIT_COLOR: Color = MARKER_COLOR.with_alpha(0.8);
const MARKER_GLOW_COLOR: Color = MARKER_COLOR.with_alpha(0.2);
const MARKER_GLOW_R: f32 = 40.0;
const MARKER_SOLID_R: f32 = 24.0;
const MARKER_SIZE: f32 = 56.0;

/// Cached ground track: its 60 SGP4 propagations are recomputed only when older
/// than [`TRACK_MAX_AGE_SECS`] (the orbit shifts only over minutes).
struct CachedTrack {
    tle: Tle,
    computed_at: f64,
    anchor: Option<f64>,
    geo: Vec<(f64, f64)>,
}

/// Max age of a cached track; over this the "now" point shifts well
/// under one track-point's spacing, so reuse is imperceptible.
const TRACK_MAX_AGE_SECS: f64 = 10.0;

thread_local! {
    static TRACK_CACHE: RefCell<Option<CachedTrack>> = const { RefCell::new(None) };
}

/// Render the globe canvas with the orbital track and centered ISS marker.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f32 canvas-geometry downcasts are intended"
)]
pub fn map_panel(data: &IssData, now_unix: f64, transition_ms: u32) -> Node {
    let mut draws: Vec<Draw> = Vec::with_capacity(16);
    let globe_zoom = orbit::globe_zoom_to_camera(GLOBE_ZOOM);

    // Prefer the live SGP4 subpoint so the globe rotates smoothly between
    // refreshes; fall back to the reported position if propagation fails.
    let mut use_anchor = false;
    let (globe_lat, globe_lon) = {
        let _s = profile::span("propagate");
        data.tle
            .as_ref()
            .and_then(|tle| {
                orbit_cache::with_orbit_model(tle, |model| model.propagate_at(now_unix)).flatten()
            })
            .unwrap_or_else(|| {
                use_anchor = true;
                (data.latitude, data.longitude)
            })
    };
    // Layer 0: textured sphere — the shader handles rotation, light shading and
    // the terminator. The host interpolates position updates between frames.
    draws.push(
        sphere!(
            &EARTH_TEXTURE,
            at: (0.0, 0.0, MAP_W, MAP_H),
            center: (globe_lat as f32, globe_lon as f32),
            zoom: globe_zoom,
            light: (data.solar_lat as f32, data.solar_lon as f32),
            atmosphere
        )
        .transition("earth-sphere", transition_ms, Easing::Linear),
    );

    // Layer 1: orbital ground track (SGP4 cached; projection follows position updates).
    if let Some(tle) = &data.tle {
        let anchor = use_anchor.then_some(data.longitude);
        let _s = profile::span("track");
        let segments = TRACK_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let stale = cache.as_ref().is_none_or(|c| {
                c.tle != *tle
                    || c.anchor != anchor
                    || (now_unix - c.computed_at).abs() > TRACK_MAX_AGE_SECS
            });
            if stale {
                *cache = Some(CachedTrack {
                    tle: tle.clone(),
                    computed_at: now_unix,
                    anchor,
                    geo: orbit_cache::with_orbit_model(tle, |model| {
                        model.ground_track(now_unix, anchor)
                    })
                    .unwrap_or_default(),
                });
            }
            let geo = &cache.as_ref().expect("BUG: populated when stale").geo;
            orbit::project_orbit_to_globe(
                geo,
                globe_lat,
                globe_lon,
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

// Copyright (C) 2026  Braiins Systems s.r.o.

//! Orbital math for the ISS globe — GMST, ECI→geodetic, SGP4 propagation,
//! ground-track computation, and projection onto the rendered 3D globe.
//!
//! Every function here is pure: it takes the current time and TLE as inputs
//! and returns geometry, so the whole orbital path is unit-testable on the
//! host without a wasm runtime or a clock.

use std::f64::consts::PI;

use crate::model::Tle;

/// ISS orbital period in minutes (well-known constant).
pub const ORBIT_PERIOD_MIN: u32 = 92;
/// Number of points to compute for the orbit ground track.
pub const ORBIT_POINTS: usize = 60;

/// Remap the user-facing zoom scale to the shader's camera distance.
///
/// `1.0` is the default full-globe view; `>1.0` zooms in, `<1.0` out. The
/// shader expects a camera distance (>1.0) in sphere radii, so the value is
/// inverted and clamped to a sane range.
#[must_use]
pub fn globe_zoom_to_camera(zoom: f32) -> f32 {
    let zoom = zoom.clamp(0.6, 1.6);
    let camera = 1.8 / zoom;
    camera.clamp(1.2, 2.6)
}

/// Greenwich Mean Sidereal Time in radians from a Unix timestamp.
#[must_use]
pub fn gmst_radians(unix_secs: f64) -> f64 {
    let jd = unix_secs / 86_400.0 + 2_440_587.5;
    let t = (jd - 2_451_545.0) / 36_525.0;
    // Vallado (4th ed.) GMST formula — result in seconds of sidereal time.
    let gmst_sec =
        67_310.548_41 + (876_600.0 * 3_600.0 + 8_640_184.812_866) * t + 0.093_104 * t * t
            - 6.2e-6 * t * t * t;
    // Normalize to [0, 86400) then convert to radians.
    let s = ((gmst_sec % 86_400.0) + 86_400.0) % 86_400.0;
    s / 86_400.0 * 2.0 * PI
}

/// Convert an ECI (TEME) position to geodetic latitude/longitude in degrees.
#[must_use]
pub fn eci_to_geodetic(pos: &[f64; 3], gmst: f64) -> (f64, f64) {
    let [x, y, z] = *pos;
    let lon_rad = y.atan2(x) - gmst;
    let lat_rad = z.atan2((x * x + y * y).sqrt());
    let lat = lat_rad.to_degrees();
    let lon = normalize_180(lon_rad.to_degrees());
    (lat, lon)
}

/// Propagate the ISS position at `now_unix` from a TLE via SGP4.
///
/// `None` if the TLE fails to parse or propagation fails.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "unix-second timestamps are exact in f64 (well below 2^53)"
)]
pub fn propagate_at(tle: &Tle, now_unix: f64) -> Option<(f64, f64)> {
    let elements =
        sgp4::Elements::from_tle(None, tle.line1.as_bytes(), tle.line2.as_bytes()).ok()?;
    let constants = sgp4::Constants::from_elements(&elements).ok()?;
    let epoch_unix = elements.datetime.and_utc().timestamp() as f64;
    let minutes_since_epoch = (now_unix - epoch_unix) / 60.0;
    let prediction = constants
        .propagate(sgp4::MinutesSinceEpoch(minutes_since_epoch))
        .ok()?;
    Some(eci_to_geodetic(
        &prediction.position,
        gmst_radians(now_unix),
    ))
}

/// Compute the ground track as geographic coordinates from a TLE via SGP4.
///
/// Returns `(lat_deg, lon_deg)` pairs for one full orbit centered on
/// `now_unix`. `anchor_center_lon` shifts the whole track so its "now" point
/// lines up with a provided longitude — used to align with the API-reported
/// position when the TLE is stale; pass `None` for unshifted SGP4 motion.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "unix-second timestamps and orbit-point indices are exact in f64 (well below 2^53)"
)]
pub fn ground_track(tle: &Tle, now_unix: f64, anchor_center_lon: Option<f64>) -> Vec<(f64, f64)> {
    let Ok(elements) = sgp4::Elements::from_tle(None, tle.line1.as_bytes(), tle.line2.as_bytes())
    else {
        return Vec::new();
    };
    let Ok(constants) = sgp4::Constants::from_elements(&elements) else {
        return Vec::new();
    };

    let epoch_unix = elements.datetime.and_utc().timestamp() as f64;
    let minutes_since_epoch = (now_unix - epoch_unix) / 60.0;

    let anchor = anchor_center_lon
        .and_then(|center_lon| {
            let t0 = sgp4::MinutesSinceEpoch(minutes_since_epoch);
            let p0 = constants.propagate(t0).ok()?;
            let (_, sgp4_lon) = eci_to_geodetic(&p0.position, gmst_radians(now_unix));
            Some(normalize_180(center_lon - sgp4_lon))
        })
        .unwrap_or(0.0);

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
        let gmst = gmst_radians(now_unix + offset * 60.0);
        let (lat, lon) = eci_to_geodetic(&prediction.position, gmst);
        points.push((lat, normalize_180(lon + anchor)));
    }
    points
}

/// Project geographic orbit points onto the 3D globe view.
///
/// Returns visible polyline segments as canvas pixel coordinates. Points on
/// the far side of the globe break the polyline into separate segments.
#[must_use]
pub fn project_orbit_to_globe(
    points: &[(f64, f64)],
    center_lat: f64,
    center_lon: f64,
    zoom: f64,
    w: f32,
    h: f32,
) -> Vec<Vec<(f32, f32)>> {
    let basis = Basis::new(center_lat, center_lon, w, h);
    let mut segments: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();

    for &(lat_deg, lon_deg) in points {
        // Back-face cull with a generous margin so Catmull-Rom smoothing on the
        // canvas side cannot overshoot the visible hemisphere.
        match basis.project(lat_deg, lon_deg, zoom, 0.40) {
            Some(pt) => current.push(pt),
            None => {
                if current.len() > 1 {
                    segments.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
        }
    }
    if current.len() > 1 {
        segments.push(current);
    }
    segments
}

/// The rotation + projection basis for one globe center, shared by the orbit
/// polyline and single-point projections so they stay in exact agreement.
struct Basis {
    cos_clat: f64,
    sin_clat: f64,
    cos_clon: f64,
    sin_clon: f64,
    aspect: f64,
    w: f32,
    h: f32,
}

impl Basis {
    fn new(center_lat: f64, center_lon: f64, w: f32, h: f32) -> Self {
        let clat = center_lat.to_radians();
        let clon = center_lon.to_radians();
        Self {
            cos_clat: clat.cos(),
            sin_clat: clat.sin(),
            cos_clon: clon.cos(),
            sin_clon: clon.sin(),
            aspect: f64::from(w) / f64::from(h),
            w,
            h,
        }
    }

    /// Project a geographic point to canvas pixels, culling points whose
    /// rotated `z` falls at or below `cull_z` (behind the visible hemisphere).
    #[expect(
        clippy::cast_possible_truncation,
        reason = "canvas geometry is f32; the pixel-coordinate downcast is intended"
    )]
    fn project(&self, lat_deg: f64, lon_deg: f64, zoom: f64, cull_z: f64) -> Option<(f32, f32)> {
        let lat = lat_deg.to_radians();
        let lon = lon_deg.to_radians();

        // Geographic → unit sphere.
        let px = lat.cos() * lon.sin();
        let py = lat.sin();
        let pz = lat.cos() * lon.cos();

        // Forward rotation Rx(clat) * Ry(-clon) — the inverse of the shader's
        // undo, so canvas overlays line up with the textured globe.
        let p1x = px * self.cos_clon - pz * self.sin_clon;
        let p1z = px * self.sin_clon + pz * self.cos_clon;
        let vx = p1x;
        let vy = py * self.cos_clat - p1z * self.sin_clat;
        let vz = py * self.sin_clat + p1z * self.cos_clat;

        if vz <= cull_z {
            return None;
        }

        // Perspective projection matching the shader's camera.
        let scale = zoom / (zoom - vz);
        let uv_x = vx * scale / self.aspect;
        let uv_y = vy * scale;

        // UV [-1, 1] → canvas pixels (Y flipped for a top-down canvas).
        let canvas_x = (f64::midpoint(uv_x, 1.0) * f64::from(self.w)) as f32;
        let canvas_y = ((1.0 - uv_y) / 2.0 * f64::from(self.h)) as f32;
        Some((canvas_x, canvas_y))
    }
}

/// Normalize a longitude (or longitude delta) of any magnitude into
/// `[-180, 180]`. `rem_euclid` keeps the result non-negative before the
/// shift, so it holds for arbitrarily large or negative input — unlike a
/// truncated `%`, which leaks the sign for input below `-540`.
fn normalize_180(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real, checksum-valid ISS TLE (NORAD 25544; epoch 2020-07-12), used to
    // exercise propagation end-to-end.
    const L1: &str = "1 25544U 98067A   20194.88612269 -.00002218  00000-0 -31515-4 0  9992";
    const L2: &str = "2 25544  51.6461 339.2086 0001413  21.0561  68.7595 15.49401438236864";
    /// A few hours after the TLE epoch.
    const NOW_UNIX: f64 = 1_594_600_000.0;

    fn tle() -> Tle {
        Tle {
            line1: L1.to_string(),
            line2: L2.to_string(),
        }
    }

    #[test]
    fn gmst_stays_within_one_turn() {
        for secs in [0.0, 1_700_000_000.0, 2_000_000_000.0] {
            let g = gmst_radians(secs);
            assert!((0.0..2.0 * PI).contains(&g), "gmst {g} out of range");
        }
    }

    #[test]
    fn zoom_remaps_and_clamps() {
        // Default zoom maps to a mid-range camera distance.
        assert!((globe_zoom_to_camera(1.0) - 1.8).abs() < 1e-6);
        // Out-of-range zoom is clamped, so the camera never inverts or runs away.
        assert!((1.2..=2.6).contains(&globe_zoom_to_camera(10.0)));
        assert!((1.2..=2.6).contains(&globe_zoom_to_camera(0.01)));
    }

    #[test]
    fn eci_to_geodetic_reads_the_equatorial_prime_meridian() {
        // +X with zero GMST is lat 0, lon 0; +Z is the north pole.
        let (lat, lon) = eci_to_geodetic(&[7000.0, 0.0, 0.0], 0.0);
        assert!(lat.abs() < 1e-6 && lon.abs() < 1e-6);
        let (lat, _) = eci_to_geodetic(&[0.0, 0.0, 7000.0], 0.0);
        assert!((lat - 90.0).abs() < 1e-6);
    }

    #[test]
    fn propagation_yields_a_plausible_subpoint() {
        let (lat, lon) = propagate_at(&tle(), NOW_UNIX).expect("BUG: real TLE propagates");
        // ISS inclination is ~51.6°, so the subpoint latitude must stay within
        // that band; longitude must be a valid wrapped value.
        assert!(lat.abs() <= 52.0, "lat {lat} exceeds ISS inclination band");
        assert!((-180.0..=180.0).contains(&lon), "lon {lon} not normalized");
    }

    #[test]
    fn ground_track_spans_one_orbit_within_the_inclination_band() {
        let track = ground_track(&tle(), NOW_UNIX, None);
        assert_eq!(track.len(), ORBIT_POINTS);
        for (lat, lon) in track {
            assert!(lat.abs() <= 52.0, "track lat {lat} exceeds band");
            assert!(
                (-180.0..=180.0).contains(&lon),
                "track lon {lon} not normalized"
            );
        }
    }

    #[test]
    fn normalize_180_wraps_any_magnitude_into_range() {
        // In-range values pass through unchanged.
        assert!(normalize_180(0.0).abs() < 1e-9);
        assert!((normalize_180(179.0) - 179.0).abs() < 1e-9);
        // The anchor delta mixes an unnormalized nexus longitude with the SGP4
        // subpoint, so it can land far outside a single ±360 wrap. Every
        // magnitude, positive or negative, must still resolve into range.
        for d in [360.0, 540.0, 700.0, -700.0, 1_000.0, -1_000.0, 1e6, -1e6] {
            let n = normalize_180(d);
            assert!(
                (-180.0..=180.0).contains(&n),
                "normalize_180({d}) = {n} out of range"
            );
        }
    }

    #[test]
    fn project_orbit_culls_the_far_side() {
        // An equatorial ring centered at (0, 0): the near hemisphere projects
        // into one or more segments, the far hemisphere is culled, so the
        // projected-point count is non-zero but below the input count.
        let ring: Vec<(f64, f64)> = (0..36)
            .map(|i| (0.0, -180.0 + f64::from(i) * 10.0))
            .collect();
        let segments = project_orbit_to_globe(&ring, 0.0, 0.0, 1.8, 560.0, 480.0);
        assert!(!segments.is_empty(), "near hemisphere must project");
        let projected: usize = segments.iter().map(Vec::len).sum();
        assert!(
            projected > 0 && projected < ring.len(),
            "expected some points culled, got {projected} of {}",
            ring.len()
        );
    }
}

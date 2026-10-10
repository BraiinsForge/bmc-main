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

//! Choosing the largest font size that still fits a box.

use bmc_ui::AutoFit;

/// Readability floor used when the caller does not specify `min_size`.
pub(crate) const DEFAULT_MIN_AUTOFIT: u32 = 12;

/// Inclusive `[lower, upper]` font-size search range for an autofit command.
/// `target_height` bounds growth when `max_size` is unset: a line at size S is
/// `S * line_height` px tall, so a fitting size never exceeds
/// `target_height / line_height`.
pub(crate) fn autofit_bounds(
    size: u32,
    min_size: u32,
    max_size: u32,
    mode: AutoFit,
    target_height: Option<f32>,
    line_height: f32,
) -> (u32, u32) {
    let floor_default = if min_size > 0 {
        min_size
    } else {
        // Never let the implicit readability floor exceed the configured `size`:
        // for a `size` below the floor, a Shrink search would otherwise be
        // clamped *up* to the floor and grow text in a shrink-only mode.
        DEFAULT_MIN_AUTOFIT.min(size.max(1))
    };
    let lower = match mode {
        AutoFit::Grow => size,
        AutoFit::Shrink | AutoFit::ShrinkAndGrow => floor_default,
    };
    let upper = match mode {
        AutoFit::Shrink => size,
        AutoFit::Grow | AutoFit::ShrinkAndGrow => {
            if max_size > 0 {
                max_size
            } else {
                target_height.map_or(size, |h| {
                    // A line at size S is `S * line_height` px tall, so the
                    // tallest size that can fit is `h / line_height`. Guard a
                    // non-positive line_height (degenerate) by falling back to
                    // `size`, which never grows the text.
                    if line_height <= 0.0 {
                        return size;
                    }
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "box height is small, finite and non-negative"
                    )]
                    let cap = (h / line_height).floor() as u32;
                    cap
                })
            }
        }
    };
    let lower = lower.max(1);
    let upper = upper.max(lower);
    (lower, upper)
}

/// Binary-search the largest integer size in `[lower, upper]` whose measured
/// layout fits both targets. `measure_at(size)` returns `(width, height)`;
/// an absent target is always satisfied. Returns `lower` if even `lower`
/// overflows. Pure; ~log2(range) calls to `measure_at`.
pub(crate) fn search_fit_size(
    lower: u32,
    upper: u32,
    target_width: Option<f32>,
    target_height: Option<f32>,
    mut measure_at: impl FnMut(u32) -> (f32, f32),
) -> u32 {
    let fits = |w: f32, h: f32| {
        target_width.is_none_or(|tw| w <= tw) && target_height.is_none_or(|th| h <= th)
    };
    let mut lo = lower;
    let mut hi = upper;
    let mut best = lower;
    while lo <= hi {
        #[expect(
            clippy::integer_division,
            reason = "binary search midpoint; truncation is correct"
        )]
        let mid = lo + (hi - lo) / 2;
        let (w, h) = measure_at(mid);
        if fits(w, h) {
            best = mid;
            lo = mid + 1;
        } else if mid == lower {
            break;
        } else {
            hi = mid - 1;
        }
    }
    best
}

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

//! Types for interaction handling.

/// Touch event from the host.
#[derive(Debug, Clone, Copy)]
pub enum TouchEvent {
    Down {
        x: f32,
        y: f32,
    },
    /// Finger lifted. Coordinates are not included because the Wayland
    /// `wl_touch::up` event does not carry them. `InteractionState` uses
    /// the last known position from the preceding `Move` or `Down` for
    /// hit-testing the release.
    Up,
    /// Current touch stream was canceled by the host compositor.
    Cancel,
    Move {
        x: f32,
        y: f32,
    },
    /// Mouse wheel scroll event. delta_y is positive for scroll down, negative for scroll up.
    Scroll {
        x: f32,
        y: f32,
        delta_y: f32,
    },
}

impl TouchEvent {
    #[must_use]
    pub fn is_motion_like(&self) -> bool {
        matches!(self, Self::Move { .. } | Self::Scroll { .. })
    }
}

/// High-level interaction event consumed during a render frame.
///
/// Recorded by `InteractionState` when widgets consume pointer input (e.g. a
/// button click is detected). Useful for dev-tool action logs.
#[derive(Debug, Clone)]
pub enum ActionEvent {
    Click {
        key: String,
        pos: Option<(f32, f32)>,
    },
    Scroll {
        key: String,
        delta: i32,
    },
}

/// Rectangle for hit testing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    #[must_use]
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Check if a point is inside the rectangle.
    #[must_use]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }

    /// Area for hit-test specificity (smaller area = more specific target).
    #[must_use]
    pub fn area(&self) -> f32 {
        self.w * self.h
    }

    /// Grow `self` to include `rect`
    pub fn union(&mut self, rect: Rect) {
        let &Rect { x, y, w, h } = &rect;
        let (x1, y1) = ((self.x + self.w).max(x + w), (self.y + self.h).max(y + h));
        self.x = self.x.min(x);
        self.y = self.y.min(y);
        self.w = x1 - self.x;
        self.h = y1 - self.y;
    }

    /// Whether the two rectangles share any area.
    #[must_use]
    pub fn overlaps(&self, other: Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    /// The area the two rectangles share, if any.
    #[must_use]
    pub fn intersection(&self, other: Rect) -> Option<Rect> {
        self.overlaps(other).then(|| {
            let (x, y) = (self.x.max(other.x), self.y.max(other.y));
            let right = (self.x + self.w).min(other.x + other.w);
            let bottom = (self.y + self.h).min(other.y + other.h);
            Rect::new(x, y, right - x, bottom - y)
        })
    }

    /// Grow `bounds` to include `rect`.
    pub fn union_bounds(bounds: &mut Option<Rect>, rect: Rect) {
        if let Some(bounds) = bounds {
            bounds.union(rect);
        } else {
            *bounds = Some(rect);
        }
    }
}

impl From<Rect> for (f32, f32, f32, f32) {
    fn from(value: Rect) -> Self {
        (value.x, value.y, value.w, value.h)
    }
}

impl From<&Rect> for (f32, f32, f32, f32) {
    fn from(value: &Rect) -> Self {
        (value.x, value.y, value.w, value.h)
    }
}

impl From<(f32, f32, f32, f32)> for Rect {
    fn from((x, y, w, h): (f32, f32, f32, f32)) -> Self {
        Self { x, y, w, h }
    }
}

pub trait Coalesce: Sized {
    /// Merge rectangles into their bounding unions
    /// for as long as `merges` approves some pair.
    #[must_use]
    fn coalesce_by(self, merges: impl Fn(Rect, Rect) -> bool) -> Self;

    /// Merge two overlapping rectangles into their bounding box only while it
    /// covers at most `tolerance` times their combined area.
    ///
    /// The damage set spans two walks, so anything still moving contributes a
    /// near-identical rect per walk. Left alone each is scissored and drawn
    /// separately, painting the same pixels twice and counting them twice
    /// against `BMC_DAMAGE_MAX_PCT` — enough on its own to cross the cap and
    /// fall back to full repaints.
    ///
    /// At 1.0 a merge never paints more than the two apart,
    /// so rects that overlap only a little but lie far apart stay apart,
    /// and the survivors may overlap.
    #[must_use]
    fn coalesce_within(self, tolerance: f32) -> Self {
        self.coalesce_by(|a, b| {
            let mut union = a;
            union.union(b);
            a.overlaps(b) && union.area() <= tolerance * (a.area() + b.area())
        })
    }
}

impl Coalesce for Vec<Rect> {
    fn coalesce_by(mut self, merges: impl Fn(Rect, Rect) -> bool) -> Self {
        let mut settled = Vec::with_capacity(self.len());
        while let Some(mut rect) = self.pop() {
            // Only a rect that grew can newly qualify for a merge,
            // so the one being grown is the only one rechecked.
            loop {
                let mut grew = false;
                let mut absorb = |other: &Rect| {
                    let merge = merges(rect, *other);
                    if merge {
                        rect.union(*other);
                        grew = true;
                    }
                    !merge
                };
                self.retain(&mut absorb);
                settled.retain(&mut absorb);
                if !grew {
                    break;
                }
            }
            settled.push(rect);
        }
        settled
    }
}

pub trait CoalesceIterExt: Iterator<Item = Rect> + Sized {
    /// [`Coalesce::coalesce_within`] over the rects this yields.
    fn coalesce_within(self, tolerance: f32) -> Vec<Rect> {
        self.collect::<Vec<_>>().coalesce_within(tolerance)
    }
}

impl<I: Iterator<Item = Rect>> CoalesceIterExt for I {}

#[cfg(test)]
mod tests {
    use super::{Coalesce, Rect};

    #[test]
    fn intersection_is_the_shared_area() {
        let shared = Rect::new(0.0, 0.0, 10.0, 10.0)
            .intersection(Rect::new(5.0, 2.0, 10.0, 4.0))
            .expect("BUG: the two rects overlap");
        assert_eq!(
            (shared.x, shared.y, shared.w, shared.h),
            (5.0, 2.0, 5.0, 4.0)
        );
    }

    /// Two rects overlapping in a 10x10 corner, under a bounding box far larger than the pair.
    fn rects_overlapping_at_a_corner() -> Vec<Rect> {
        vec![
            Rect::new(0.0, 0.0, 110.0, 110.0),
            Rect::new(100.0, 100.0, 100.0, 100.0),
        ]
    }

    #[test]
    fn coalesce_by_merges_every_pair_its_predicate_approves() {
        assert_eq!(
            rects_overlapping_at_a_corner()
                .coalesce_by(|a, b| a.overlaps(b))
                .len(),
            1
        );
    }

    #[test]
    fn coalesce_within_keeps_apart_rects_whose_union_costs_more() {
        assert_eq!(
            rects_overlapping_at_a_corner().coalesce_within(1.0).len(),
            2,
            "a 200x200 union paints far more than the two 110x110 and 100x100 rects"
        );
    }

    #[test]
    fn coalesce_within_merges_near_duplicates() {
        let rects = vec![
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Rect::new(2.0, 0.0, 100.0, 100.0),
        ];
        assert_eq!(rects.coalesce_within(1.0).len(), 1);
    }

    #[test]
    fn a_rect_grown_by_a_merge_absorbs_what_it_now_reaches() {
        let chain: Vec<_> = (0..50_u8)
            .map(|step| Rect::new(f32::from(step) * 9.0, 0.0, 10.0, 10.0))
            .collect();
        assert_eq!(chain.coalesce_by(|a, b| a.overlaps(b)).len(), 1);
    }

    /// 256 draws on a grid, each reported by two walks a pixel apart.
    fn two_walks_of_a_grid() -> Vec<Rect> {
        (0..16_u8)
            .flat_map(|row| {
                (0..16_u8).flat_map(move |column| {
                    let (x, y) = (f32::from(column) * 20.0, f32::from(row) * 20.0);
                    [
                        Rect::new(x, y, 10.0, 10.0),
                        Rect::new(x + 1.0, y, 10.0, 10.0),
                    ]
                })
            })
            .collect()
    }

    #[test]
    fn merging_two_walks_of_many_draws_checks_each_pair_a_bounded_number_of_times() {
        let rects = two_walks_of_a_grid();
        let count = rects.len();
        let checks = std::cell::Cell::new(0_usize);
        let merged = rects.coalesce_by(|a, b| {
            checks.set(checks.get() + 1);
            a.overlaps(b)
        });
        assert_eq!(
            merged.len(),
            256,
            "each draw's two walks merge, nothing else"
        );
        assert!(
            checks.get() <= count * count,
            "{} checks for {count} rects, past the {} of a quadratic merge",
            checks.get(),
            count * count
        );
    }

    #[test]
    fn a_loose_enough_tolerance_merges_any_overlap() {
        assert_eq!(
            rects_overlapping_at_a_corner().coalesce_within(2.0).len(),
            1
        );
    }

    #[test]
    fn rects_that_only_touch_have_no_intersection() {
        assert!(
            Rect::new(0.0, 0.0, 10.0, 10.0)
                .intersection(Rect::new(10.0, 0.0, 5.0, 5.0))
                .is_none()
        );
    }
}

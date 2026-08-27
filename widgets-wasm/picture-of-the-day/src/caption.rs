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

//! The caption drawn over the picture: the title top-left, the credit bottom-right.
//! Fixed type size and fixed inset, drawn at every supported viewport
//! from the 317×238 tile up to the 1280×480 fullscreen.

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

pub const TITLE_PX: u32 = 16;
pub const CREDIT_PX: u32 = 12;

/// The credit has to read as secondary to the title at a glance.
const _: () = assert!(CREDIT_PX < TITLE_PX);

/// The title's inset from the viewport edge, matching the inset the SDK's
/// status-overlay tags use so the title and a stale tag sit on one grid.
/// Half of it is the credit plate's own padding; the plate is flush in its
/// corner rather than inset, so it is not on that grid.
pub const PADDING_PX: u32 = 8;

/// Longest credit line, as a percentage of the viewport width.
/// Past this the plate would reach across the picture,
/// so the text wraps onto another line instead.
const CREDIT_WIDTH_PERCENT: u32 = 60;

/// Width the credit wraps at, in pixels.
#[must_use]
pub fn credit_max_width(width: u32) -> u32 {
    width * CREDIT_WIDTH_PERCENT / 100
}

/// Dark plate behind the credit, so a name stays readable over a bright sky.
#[cfg(target_arch = "wasm32")]
const CREDIT_BACKDROP_ALPHA: f32 = 0.55;

/// The title has no plate of its own, so it carries a dark outline.
#[cfg(target_arch = "wasm32")]
const TITLE_OUTLINE_ALPHA: f32 = 0.75;
#[cfg(target_arch = "wasm32")]
const TITLE_OUTLINE_WIDTH: f32 = 2.0;

/// Overlay the caption onto the picture.
///
/// Compose this **before** the tap-to-reveal menu:
/// the caption must sit under the menu's tap catcher,
/// or a tap on the title would not open the menu.
#[cfg(target_arch = "wasm32")]
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "viewport dimensions are < 2^24 and exact in f32"
)]
pub fn with_caption(mut root: Node, title: Option<&str>, credit: &str, size: WidgetSize) -> Node {
    let pad = PADDING_PX as f32;
    let (Node::Column(_, children) | Node::Row(_, children)) = &mut root else {
        return root;
    };

    // `style!` carries no insets, so the absolute placement lives on a wrapper.
    //
    // Both nodes below pin `text_overflow` even though `Wrap` is the default:
    // the renderer honours `max_width` only in the wrap branch, so `Ellipsis`
    // would drop the cap, letting the title run off the edge
    // and the credit plate reach across the picture.
    if let Some(title) = title.filter(|t| !t.is_empty()) {
        children.push(row(
            props!(inset_top: pad, inset_left: pad),
            [text(
                title.to_string(),
                style!(
                    size: TITLE_PX,
                    weight: FontWeight::SEMIBOLD,
                    color: WHITE,
                    line_height: 1.1,
                    max_width: size.width.saturating_sub(PADDING_PX * 2),
                    outline_color: BLACK.with_alpha(TITLE_OUTLINE_ALPHA),
                    outline_width: TITLE_OUTLINE_WIDTH,
                    text_overflow: TextOverflow::Wrap
                ),
            )],
        ));
    }

    // The credit is never optional:
    // these pictures are frequently the photographer's copyright, not NASA's.
    if !credit.is_empty() {
        let max_text = credit_max_width(size.width);
        // Flush into the corner, square: a rounded plate on the panel edge
        // leaves slivers of picture showing past its two outer corners.
        children.push(row(
            props!(
                background: BLACK.with_alpha(CREDIT_BACKDROP_ALPHA),
                padding: pad * 0.5,
                inset_bottom: 0.0,
                inset_right: 0.0
            ),
            [text(
                fmt!("© {credit}"),
                style!(
                    size: CREDIT_PX,
                    weight: FontWeight::REGULAR,
                    color: GRAY_30,
                    line_height: 1.1,
                    align: TextAlign::Right,
                    max_width: max_text,
                    text_overflow: TextOverflow::Wrap
                ),
            )],
        ));
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_credit_band_leaves_most_of_the_picture_clear() {
        assert_eq!(credit_max_width(1280), 768);
        assert!(credit_max_width(317) < 317);
    }

    #[test]
    fn the_title_fits_inside_the_narrowest_tile() {
        // bmc100:small is the narrowest rectangular viewport a widget can occupy.
        assert!(
            317_u32.saturating_sub(PADDING_PX * 2) > TITLE_PX * 4,
            "a 317px tile must hold more than a few characters of title"
        );
    }
}

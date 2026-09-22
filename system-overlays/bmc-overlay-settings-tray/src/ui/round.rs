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

//! The BFM100's arrangement: one column inside the disc's chord-safe band.

use super::parts::{header_row, pad_horizontal};
use super::station::station_line;
use super::{
    Content, LINE_H, NO_DATA_PLACEHOLDER, Panel, ROUND_BOTTOM_GAP, ROUND_CONTROLS_TOP, ROUND_H_PAD,
    ROUND_HEADER_WIDTH, ROUND_TOP_GAP, Tier, caption_slot, control_row_nodes,
};
use bmc_render::tree::{TreeNode, fixed_height, spacer};

/// The BFM100's flow children: one column inside the disc's chord-safe band,
/// with the control rows pinned to a fixed top edge.
#[expect(clippy::cast_precision_loss, reason = "display sizes are small")]
pub(super) fn round_children(content: Content<'_>, panel: Panel, tier: Tier) -> Vec<TreeNode> {
    let header_h = tier.hostname_size as f32 * LINE_H;
    let mut children = vec![
        fixed_height(ROUND_TOP_GAP),
        pad_horizontal(
            header_row(
                content.ip.unwrap_or(NO_DATA_PLACEHOLDER),
                tier.hostname_size,
            ),
            (panel.width as f32 - ROUND_HEADER_WIDTH) / 2.0,
        ),
        // Pin the control rows below the chord-safe close target.
        fixed_height(ROUND_CONTROLS_TOP - ROUND_TOP_GAP - header_h),
    ];
    for row_node in control_row_nodes(content, tier) {
        children.push(pad_horizontal(row_node, ROUND_H_PAD));
        children.push(fixed_height(tier.row_gap));
    }
    children.push(pad_horizontal(caption_slot(content, tier), ROUND_H_PAD));
    children.push(spacer(1.0));
    children.push(pad_horizontal(station_line(content, tier), ROUND_H_PAD));
    children.push(fixed_height(ROUND_BOTTOM_GAP));
    children
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::parts::close_origin;
    use crate::ui::test_support::*;
    use crate::ui::*;

    #[test]
    fn round_close_target_stays_inside_the_disc_and_above_the_controls() {
        // Round-panel chord safety: the far corner of the close target must
        // stay inside the disc.
        let panel = round_panel();
        let tier = tier_for(&panel);
        let (left, top) = close_origin(&panel, tier);
        let r = 240.0_f32;
        let far_x = left + CLOSE_TARGET - r;
        let far_y = top - r;
        let dist = (far_x * far_x + far_y * far_y).sqrt();
        assert!(
            dist < r,
            "close target far corner at {dist} must stay inside the 240px disc"
        );
        assert!(
            ROUND_CONTROLS_TOP >= top + CLOSE_TARGET,
            "round control rows start below the close target"
        );
    }
}

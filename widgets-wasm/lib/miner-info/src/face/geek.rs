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

//! The Geek face on the rectangular panels: the network's figures, one line
//! each, from the BMM101 frame; the small panel gets the same list at its own
//! type size.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use bmc_wasm_sdk::types::Ratio;

use super::{
    BACKGROUND, TITLE, VALUE, change_color, fixed_height, fixed_width, icons, rule, text_line,
    title_row, unit_visible, value_with_unit, with_horizontal_padding,
};
use crate::format;
use crate::layout::{self, Panel};
use crate::model::{Availability, PublicData};

const ETA_GAP: f32 = 16.0;
const ADJUSTMENT_DECIMALS: u32 = 1;
const TAG_PADDING: f32 = 2.0;
// The frame pads the tag 4 px across and 2 px down; `padding` is one number.
const TAG_INSET: f32 = 2.0;
const TAG_RADIUS: f32 = 4.0;
// Tint over black, as the ticker badges do, rather than the frame's own fills.
const TAG_BACKGROUND_ALPHA: f32 = 0.15;

fn label(name: &'static str, sizes: layout::TextSizes) -> Node {
    text(
        name,
        style!(size: sizes.title, weight: FontWeight::SEMIBOLD, color: TITLE, flex: 1.0),
    )
}

// The retarget ETA in the label's voice, the progress in the value's.
// An unknown ETA is left out, so the line reads one `N/A`, not two.
fn epoch_line(public: &PublicData, sizes: layout::TextSizes) -> Node {
    let mut parts = vec![label("Epoch Progress", sizes)];
    if let Availability::Available(remaining) = public.epoch_remaining {
        parts.push(text(
            format::epoch_eta(remaining),
            style!(size: sizes.value, weight: FontWeight::REGULAR, color: TITLE),
        ));
        parts.push(fixed_width(ETA_GAP));
    }
    parts.push(value_with_unit(
        format::fixed(public.epoch_progress, 0),
        sizes,
        TextAlign::Right,
        VALUE,
        FontWeight::REGULAR,
    ));
    row(props!(cross_align: CrossAlign::Center), parts)
}

// A signed change in a tinted pill; a placeholder stays plain, like an affix.
fn adjustment_line(
    name: &'static str,
    change: Availability<Ratio>,
    sizes: layout::TextSizes,
) -> Node {
    let rendered = format::signed_percent_unit(change, ADJUSTMENT_DECIMALS);
    let known = unit_visible(&rendered);
    let color = change_color(change);
    let value = text(
        rendered,
        style!(size: sizes.value, weight: FontWeight::BOLD, color: color),
    );
    let trailing = if known {
        row(
            props!(
                background: color.with_alpha(TAG_BACKGROUND_ALPHA),
                border_radius: TAG_RADIUS,
                padding: TAG_PADDING
            ),
            [fixed_width(TAG_INSET), value, fixed_width(TAG_INSET)],
        )
    } else {
        value
    };
    row(
        props!(cross_align: CrossAlign::Center),
        [label(name, sizes), trailing],
    )
}

#[must_use]
pub fn geek(panel: Panel, public: &PublicData) -> Node {
    let metrics = layout::geek_layout(panel);
    let sizes = metrics.text;
    let lines = [
        text_line(
            "Network HR",
            format::network_hashrate(public.network_hashrate),
            sizes,
        ),
        text_line(
            "Block Height",
            format::public_integer(public.block_height),
            sizes,
        ),
        epoch_line(public, sizes),
        adjustment_line("Diff. Adjustment", public.prev_diff_adjust, sizes),
        adjustment_line("Est. Diff. Adjustment", public.est_diff_adjust, sizes),
        text_line(
            "Fees (144 Blocks)",
            format::fees(public.avg_fees_per_block, public.avg_fee_share),
            sizes,
        ),
        text_line(
            "Hashvalue",
            format::fixed_strip_zero_fraction(public.hashvalue, 2),
            sizes,
        ),
    ];

    // Lines and rules spread over the height, the frame's `justify-between`.
    let mut rows = Vec::with_capacity(lines.len() * 4);
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            rows.push(spacer(1.0));
            rows.push(with_horizontal_padding(rule(), metrics.edge));
            rows.push(spacer(1.0));
        }
        rows.push(with_horizontal_padding(line, metrics.edge));
    }

    col(
        props!(background: BACKGROUND),
        [
            fixed_height(metrics.edge),
            with_horizontal_padding(title_row(&icons::GEEK, "Miner Info - Geek"), metrics.edge),
            fixed_height(metrics.title_to_rows),
            col(props!(flex: 1.0), rows),
            fixed_height(metrics.edge),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::face::test_support::texts;
    use crate::fixtures::{PriceMove, Reported, public};

    #[test]
    fn an_unknown_network_reads_one_placeholder_per_line() {
        assets::init_test_registrars();
        let texts = texts(&geek(
            Panel::Bmm101,
            &public(Reported::Nothing, PriceMove::Up),
        ));
        let placeholders = texts.iter().filter(|text| *text == "N/A").count();
        assert_eq!(
            placeholders, 7,
            "one N/A per line, the epoch ETA left out: {texts:?}"
        );
    }

    #[test]
    fn a_known_adjustment_wears_a_pill_and_a_placeholder_does_not() {
        let sizes = layout::geek_layout(Panel::Bmm101).text;
        let known = adjustment_line(
            "Diff. Adjustment",
            Availability::Available(Ratio::from_percent(-4.5)),
            sizes,
        );
        let Node::Row(_, children) = known else {
            panic!("BUG: a line is a row");
        };
        assert!(
            matches!(&children[1], Node::Row(props, _) if props.border_radius > 0.0),
            "the value sits in a rounded pill"
        );

        let placeholder = adjustment_line("Diff. Adjustment", Availability::Unavailable, sizes);
        let Node::Row(_, children) = placeholder else {
            panic!("BUG: a line is a row");
        };
        assert!(
            matches!(&children[1], Node::Paragraph { .. }),
            "N/A stays plain text"
        );
    }
}

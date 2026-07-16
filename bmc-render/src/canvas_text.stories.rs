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

use crate::prelude::*;

story_meta! { title: "Canvas/Text" }

#[story(default)]
fn styles(c: &mut StoryCtx) {
    c.ui.header(
        "Canvas Text",
        "Draw::text with various styles and alignment",
    );

    c.ui.div(
        (400, 120),
        canvas(
            props!(width: 400, height: 120),
            [
                Draw::text(10.0, 10.0, "Default (14px)", style!(size: 14, color: WHITE)),
                Draw::text(10.0, 30.0, "Small (10px)", style!(size: 10, color: GRAY_40)),
                Draw::text(
                    10.0,
                    48.0,
                    "Bold 20px",
                    style!(size: 20, weight: FontWeight::BOLD, color: GREEN_50),
                ),
                Draw::text(
                    10.0,
                    76.0,
                    "Italic",
                    style!(size: 16, color: VIOLET_50, italic: true),
                ),
                Draw::text(
                    390.0,
                    100.0,
                    "Right-aligned",
                    style!(size: 12, color: ORANGE_50, align: TextAlign::Right),
                ),
            ],
        ),
    );
}

#[story]
fn alignment(c: &mut StoryCtx) {
    c.ui.header("Text Alignment", "Left, Center, Right within canvas");

    // Vertical guide lines at left edge, center, right edge.
    c.ui.div(
        (300, 100),
        canvas(
            props!(width: 300, height: 100),
            [
                // Guide lines
                Draw::rect(0.0, 0.0, 1.0, 100.0, GRAY_80),
                Draw::rect(150.0, 0.0, 1.0, 100.0, GRAY_80),
                Draw::rect(299.0, 0.0, 1.0, 100.0, GRAY_80),
                // Left aligned (default)
                Draw::text(0.0, 10.0, "Left", style!(size: 14, color: WHITE)),
                // Center aligned
                Draw::text(
                    150.0,
                    40.0,
                    "Center",
                    style!(size: 14, color: BLUE_50, align: TextAlign::Center),
                ),
                // Right aligned
                Draw::text(
                    300.0,
                    70.0,
                    "Right",
                    style!(size: 14, color: RED_50, align: TextAlign::Right),
                ),
            ],
        ),
    );
}

#[story]
fn vertical_alignment(c: &mut StoryCtx) {
    c.ui.header(
        "Vertical Alignment",
        "Top / Center / Bottom / Baseline anchored at the same `y`",
    );

    // Horizontal guide line at y=50. Each label is anchored at that y;
    // visible position differs by vertical_align.
    c.ui.div(
        (480, 100),
        canvas(
            props!(width: 480, height: 100),
            [
                Draw::rect(0.0, 50.0, 480.0, 1.0, GRAY_80),
                Draw::text(
                    10.0,
                    50.0,
                    "Top",
                    style!(size: 16, color: WHITE, valign: VerticalAlign::Top),
                ),
                Draw::text(
                    130.0,
                    50.0,
                    "Center",
                    style!(size: 16, color: BLUE_50, valign: VerticalAlign::Center),
                ),
                Draw::text(
                    260.0,
                    50.0,
                    "Bottom",
                    style!(size: 16, color: RED_50, valign: VerticalAlign::Bottom),
                ),
                Draw::text(
                    380.0,
                    50.0,
                    "Baseline",
                    style!(size: 16, color: GREEN_50, valign: VerticalAlign::Baseline),
                ),
            ],
        ),
    );
}

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

use bmc_gallery::prelude::*;

scene_meta! { title: "Components / Typography / Overflow" }

const OVERWIDE: &str = "Grayscale Bitcoin Mini Trust ETF";

const MODES: [(&str, TextOverflow); 3] = [
    ("Wrap", TextOverflow::Wrap),
    ("Clip", TextOverflow::Clip),
    ("Ellipsis", TextOverflow::Ellipsis),
];

/// Width of one example: three abreast fill the page.
const EXAMPLE_W: f32 = 400.0;

/// Saturated, so a box's edges read against the stage's checkerboard.
const BOX: Color = BLUE_50;

fn caption(label: &str) -> Node {
    text(label, style!(size: 14, color: GRAY_50))
}

/// `content` in a shaded box of `width`, so the box's edges show.
fn boxed(width: f32, content: Node) -> Node {
    col(props!(width: width, background: BOX), [content])
}

fn example(label: &str, content: Node) -> Node {
    col(props!(width: EXAMPLE_W, gap: 6), [caption(label), content])
}

fn abreast(examples: [Node; 3]) -> Node {
    row(props!(gap: 16, cross_align: CrossAlign::Start), examples)
}

/// One over-wide value per box, in every mode and alignment.
fn mode_grid(width: f32) -> Node {
    let rows = MODES.map(|(name, overflow)| {
        let cells = [TextAlign::Left, TextAlign::Center, TextAlign::Right].map(|align| {
            boxed(
                width,
                text(
                    OVERWIDE,
                    style!(size: 20, color: WHITE, align: align, text_overflow: overflow),
                ),
            )
        });
        row(
            props!(gap: 16, cross_align: CrossAlign::Start),
            [col(props!(width: 80), [caption(name)])]
                .into_iter()
                .chain(cells),
        )
    });
    col(props!(gap: 12), rows)
}

/// How cut lines share a row that runs out of room.
fn in_a_row(width: f32) -> [Node; 3] {
    [
        example(
            "Label and value, both ellipsized, share the shrink",
            row(
                props!(width: width, gap: 8, background: BOX, justify_content: Justify::SpaceBetween),
                [
                    text(
                        "Network difficulty",
                        style!(size: 20, color: BLUE_100, text_overflow: TextOverflow::Ellipsis),
                    ),
                    text(
                        "129.44 T",
                        style!(size: 20, color: WHITE, align: TextAlign::Right, text_overflow: TextOverflow::Ellipsis),
                    ),
                ],
            ),
        ),
        example(
            "A yielding run: only the flex-1 row gives way",
            row(
                props!(width: width, gap: 8, background: BOX),
                [
                    text("Account", style!(size: 20, color: BLUE_100)),
                    row(
                        props!(flex: 1.0),
                        [
                            text("(", style!(size: 20, color: WHITE)),
                            text(
                                "a-very-long-account-name",
                                style!(size: 20, color: WHITE, text_overflow: TextOverflow::Ellipsis),
                            ),
                            text(")", style!(size: 20, color: WHITE)),
                        ],
                    ),
                ],
            ),
        ),
        example(
            "Two clipped siblings",
            row(
                props!(width: width, gap: 8, background: BOX),
                [
                    text(
                        OVERWIDE,
                        style!(size: 20, color: WHITE, text_overflow: TextOverflow::Clip),
                    ),
                    text(
                        OVERWIDE,
                        style!(size: 20, color: BLUE_100, text_overflow: TextOverflow::Clip),
                    ),
                ],
            ),
        ),
    ]
}

/// What a cut does to the line itself: its spans, its cap and its line box.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the knob is a whole-pixel width"
)]
fn in_the_line(width: f32) -> [Node; 3] {
    [
        example(
            "Mixed sizes: the … takes the first span's style",
            boxed(
                width,
                paragraph(
                    style!(size: 40, weight: FontWeight::SEMIBOLD, color: WHITE, line_height: 1.0, text_overflow: TextOverflow::Ellipsis),
                    [
                        span("21", ()),
                        span(
                            " million BTC, the supply cap",
                            style!(size: 20, color: BLUE_100),
                        ),
                    ],
                ),
            ),
        ),
        example(
            "max_width caps the box a stretched text aligns in",
            col(
                props!(width: EXAMPLE_W, background: BOX),
                [
                    text(
                        OVERWIDE,
                        style!(size: 20, color: WHITE, max_width: width as u32, text_overflow: TextOverflow::Ellipsis),
                    ),
                    text(
                        "129.44 T",
                        style!(size: 20, color: BLUE_100, align: TextAlign::Right, max_width: width as u32, text_overflow: TextOverflow::Ellipsis),
                    ),
                ],
            ),
        ),
        example(
            "line_height 1.0: descenders and accents outlive the cut",
            boxed(
                width,
                text(
                    "Ágjpqy gjpqy gjpqy gjpqy",
                    style!(size: 40, color: WHITE, line_height: 1.0, text_overflow: TextOverflow::Clip),
                ),
            ),
        ),
    ]
}

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    let width = ctx.slider("Box width", 160.0, 16.0, 360.0, 1.0);
    ui.label("Every example is squeezed into the box width");
    ctx.node_stage(ui, Page, || {
        col(
            props!(gap: 24, padding: 16),
            [
                mode_grid(width),
                abreast(in_a_row(width)),
                abreast(in_the_line(width)),
            ],
        )
    });
}

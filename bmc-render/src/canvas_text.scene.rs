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

scene_meta! { title: "Components / Canvas / Text" }

#[scene(default)]
fn styles(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Canvas Text");
    ui.label("Draw::text with various styles and alignment");

    ctx.node_stage(ui, (400_u32, 120_u32), || {
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
        )
    });
}

/// Every text path the glyph cache has to serve, in one stage:
/// a wrapped paragraph of decorated and coloured spans, an outlined headline,
/// three overlapping translucent draws, a run the canvas scissor cuts,
/// and a curved run.
///
/// The headline size is a knob because 92 px is where text stops coming
/// from the cache and femtovg path-renders it instead;
/// a capture either side of that covers both.
#[scene]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the slider is a whole-pixel font size"
)]
fn glyph_cache(ctx: &mut SceneCtx, ui: &mut Ui) {
    let headline = ctx.slider("Headline size", 40.0, 8.0, 120.0, 1.0) as u32;

    ui.heading("Glyph Cache Text");
    ui.label("Paragraph, decorations, outline, scissor, overlap and curve");

    ctx.node_stage(ui, (460_u32, 500_u32), || {
        col(
            props!(gap: 10, padding: 10),
            [
                paragraph(
                    style!(size: 16, color: GRAY_10, line_height: 1.4, max_width: 440),
                    [
                        span("Cached glyphs wrap across lines, ", ()),
                        span("bold", style!(weight: FontWeight::BOLD)),
                        span(" and ", ()),
                        span("italic", style!(italic: true)),
                        span(" and ", ()),
                        span("underlined", style!(underline: true, color: GREEN_50)),
                        span(" and ", ()),
                        span("struck", style!(strikethrough: true, color: RED_50)),
                        span(" share one line.", ()),
                    ],
                ),
                canvas(
                    props!(width: 440, height: 420, background: Color::from_hex(0x0B1016)),
                    [
                        Draw::text(
                            8.0,
                            4.0,
                            "Outlined",
                            style!(
                                size: headline,
                                weight: FontWeight::BOLD,
                                color: WHITE,
                                outline_color: BLUE_50,
                                outline_width: 3.0,
                            ),
                        ),
                        // Translucent and overlapping, so these pixels record
                        // the batch order.
                        Draw::text(
                            8.0,
                            140.0,
                            "OVER",
                            style!(size: 56, color: Color::from_rgba(0xF0, 0x40, 0x40, 0xC0)),
                        ),
                        Draw::text(
                            60.0,
                            156.0,
                            "LAP",
                            style!(size: 56, color: Color::from_rgba(0x40, 0xF0, 0x60, 0xC0)),
                        ),
                        Draw::text(
                            112.0,
                            172.0,
                            "PING",
                            style!(size: 56, color: Color::from_rgba(0x50, 0x90, 0xFF, 0xC0)),
                        ),
                        // Runs off the right edge, where the canvas scissor
                        // cuts it mid-glyph.
                        Draw::text(
                            300.0,
                            240.0,
                            "scissored at the edge",
                            style!(size: 20, color: ORANGE_50, underline: true),
                        ),
                        Draw::curved_text(
                            220.0,
                            330.0,
                            76.0,
                            0.0,
                            ArcAnchor::Center,
                            ArcTextFacing::Outward,
                            "CURVED GLYPH RUN",
                            style!(size: 20, color: VIOLET_50),
                        ),
                    ],
                ),
            ],
        )
    });
}

#[scene]
fn alignment(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Text Alignment");
    ui.label("Left, Center, Right within canvas");

    // Vertical guide lines at left edge, center, right edge.
    ctx.node_stage(ui, (300_u32, 100_u32), || {
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
        )
    });
}

#[scene]
fn vertical_alignment(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Vertical Alignment");
    ui.label("Top / Center / Bottom / Baseline anchored at the same `y`");

    // Horizontal guide line at y=50. Each label is anchored at that y;
    // visible position differs by vertical_align.
    ctx.node_stage(ui, (480_u32, 100_u32), || {
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
        )
    });
}

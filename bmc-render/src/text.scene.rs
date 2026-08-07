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

scene_meta! { title: "Components / Typography" }

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.node_stage(ui, Page, || {
        col(
            props!(gap: 24, padding: 16),
            [
                // ── Type scale ──
                text("Type scale", style!(size: 14, color: GRAY_30)),
                col(
                    props!(gap: 8),
                    [
                        text(
                            "Heading 1",
                            style!(size: 32, weight: FontWeight::BOLD, color: WHITE),
                        ),
                        text(
                            "Heading 2",
                            style!(size: 24, weight: FontWeight::BOLD, color: WHITE),
                        ),
                        text(
                            "Heading 3",
                            style!(size: 20, weight: FontWeight::SEMIBOLD, color: WHITE),
                        ),
                        text("Body text (16px)", style!(size: 16, color: GRAY_10)),
                        text("Caption text (13px)", style!(size: 13, color: GRAY_30)),
                        text("Small text (11px)", style!(size: 11, color: GRAY_50)),
                    ],
                ),
                // ── Rich text ──
                text("Rich text", style!(size: 14, color: GRAY_30)),
                col(
                    props!(gap: 12),
                    [
                        paragraph(
                            style!(size: 16, color: GRAY_10, line_height: 1.4),
                            [
                                span("This is ", ()),
                                span("bold", style!(weight: FontWeight::BOLD)),
                                span(" and this is ", ()),
                                span("italic", style!(italic: true)),
                                span(".", ()),
                            ],
                        ),
                        paragraph(
                            style!(size: 16, color: GRAY_10, line_height: 1.4),
                            [
                                span("Underline", style!(underline: true)),
                                span(" / ", ()),
                                span("Strikethrough", style!(strikethrough: true)),
                                span(" / ", ()),
                                span("Colored", style!(color: VIOLET_40)),
                            ],
                        ),
                    ],
                ),
            ],
        )
    });
}

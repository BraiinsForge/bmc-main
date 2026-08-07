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

scene_meta! { title: "Components / Layout" }

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.node_stage(ui,Page, || {
        col(
            props!(gap: 24, padding: 16),
            [
                // ── Row with spacer ──
                text("Row with spacer", style!(size: 14, color: GRAY_30)),
                row(
                    props!(gap: 8, width: 300, height: 60, cross_align: CrossAlign::Center, background: GRAY_90, padding: 8),
                    [
                        text(
                            "Left",
                            style!(size: 16, color: WHITE, background: GRAY_80, padding: 8),
                        ),
                        spacer(1),
                        text(
                            "Right",
                            style!(size: 16, color: WHITE, background: GRAY_80, padding: 8),
                        ),
                    ],
                ),
                // ── Nested panels ──
                text("Nested columns", style!(size: 14, color: GRAY_30)),
                col(
                    props!(gap: 12, width: 400),
                    [
                        text(
                            "Header",
                            style!(size: 20, weight: FontWeight::BOLD, color: WHITE),
                        ),
                        row(
                            props!(gap: 8),
                            [
                                col(
                                    props!(gap: 4, flex: 1, background: GRAY_90, padding: 8),
                                    [
                                        text("Panel A", style!(size: 14, color: GRAY_30)),
                                        text("Content here", style!(size: 16, color: WHITE)),
                                    ],
                                ),
                                col(
                                    props!(gap: 4, flex: 1, background: GRAY_90, padding: 8),
                                    [
                                        text("Panel B", style!(size: 14, color: GRAY_30)),
                                        text("More content", style!(size: 16, color: WHITE)),
                                    ],
                                ),
                            ],
                        ),
                    ],
                ),
                // ── Cross-axis alignment ──
                text("Cross-axis alignment", style!(size: 14, color: GRAY_30)),
                row(
                    props!(gap: 8, width: 400, height: 80, background: GRAY_90, padding: 8, cross_align: CrossAlign::Start),
                    [text(
                        "Start",
                        style!(size: 14, color: WHITE, background: GRAY_80, padding: 4),
                    )],
                ),
                row(
                    props!(gap: 8, width: 400, height: 80, background: GRAY_90, padding: 8, cross_align: CrossAlign::Center),
                    [text(
                        "Center",
                        style!(size: 14, color: WHITE, background: GRAY_80, padding: 4),
                    )],
                ),
                row(
                    props!(gap: 8, width: 400, height: 80, background: GRAY_90, padding: 8, cross_align: CrossAlign::End),
                    [text(
                        "End",
                        style!(size: 14, color: WHITE, background: GRAY_80, padding: 4),
                    )],
                ),
                // ── Centered container ──
                text(
                    "center() — children laid out in the centre of the box",
                    style!(size: 14, color: GRAY_30),
                ),
                center(
                    props!(width: 400, height: 100, background: GRAY_90),
                    [text(
                        "Centered",
                        style!(size: 18, color: WHITE, background: GRAY_80, padding: 8),
                    )],
                ),
            ],
        )
    });
}

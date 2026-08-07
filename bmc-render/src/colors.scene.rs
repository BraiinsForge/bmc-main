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

scene_meta! { title: "Components / Colors" }

#[scene(default)]
fn palette(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.node_stage(ui,Page, || {
        col(
            props!(gap: 4, padding: 8),
            PALETTE
                .iter()
                .map(|swatch| {
                    col(
                        props!(gap: 4),
                        [
                            text(swatch.name, style!(size: 12, color: GRAY_30)),
                            row(
                                props!(gap: 2),
                                swatch
                                    .colors
                                    .iter()
                                    .enumerate()
                                    .map(|(i, &c)| {
                                        let step = (i + 1) * 10;
                                        let label_color = if step <= 50 { BLACK } else { WHITE };
                                        col(
                                            props!(width: 32, height: 32, background: c, padding: 2),
                                            [text(
                                                format!("{step}"),
                                                style!(size: 9, color: label_color),
                                            )],
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                        ],
                    )
                })
                .collect::<Vec<_>>(),
        )
    });
}

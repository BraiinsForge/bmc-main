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

scene_meta! { title: "Components / Controls / Touchable" }

const KEY: &str = "canvas";

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    let fired = ctx.node_stage_input(ui, Page, || {
        col(
            props!(gap: 24, padding: 16),
            [
                text(
                    "Touchable canvas (click the shapes)",
                    style!(size: 14, color: GRAY_30),
                ),
                touchable(
                    KEY,
                    props!(width: 200, height: 120, background: GRAY_90),
                    [
                        Draw::rect(16.0, 16.0, 80.0, 88.0, VIOLET_60),
                        Draw::circle(150.0, 60.0, 40.0, TEAL_50),
                    ],
                ),
            ],
        )
    });

    if fired.clicked(KEY) {
        action("Canvas clicked");
    }
}

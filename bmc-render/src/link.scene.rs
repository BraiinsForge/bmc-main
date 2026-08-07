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

scene_meta! { title: "Components / Controls / Link" }

const FLEET_KEY: &str = "link::fleet";
const MODEL_KEY: &str = "link::model";

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Link");
    ui.label("Tappable text whose hit target hugs the label — click a link to log it");

    let fired = ctx.node_stage_input(ui, (560_usize, AutoH), || {
        // A breadcrumb-style path: because each link hugs its label, the "/"
        // separators sit an even gap apart whatever the label lengths.
        row(
            props!(gap: 12, padding: 24, cross_align: CrossAlign::Center),
            [
                link(FLEET_KEY, "My Fleet", style!(size: 24, color: BLUE_50)),
                text("/", style!(size: 24, color: GRAY_50)),
                link(
                    MODEL_KEY,
                    "Braiins Forge Miner x4",
                    style!(size: 24, color: BLUE_50),
                ),
                text("/", style!(size: 24, color: GRAY_50)),
                text("bos-01", style!(size: 24, color: GRAY_10)),
            ],
        )
    });

    for (key, what) in [(FLEET_KEY, "fleet"), (MODEL_KEY, "model")] {
        if fired.clicked(key) {
            action(what);
        }
    }
}

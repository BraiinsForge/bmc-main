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

use std::f32::consts::PI;

use crate::prelude::*;

story_meta! { title: "Canvas/Curved Text" }

#[story(default)]
fn interactive(c: &mut StoryCtx) {
    let text = c.text("Text", "BRAIINS DECK");
    let angle = c.slider("Angle", 0.0, -PI, PI).get();
    let radius = c.slider("Radius", 126.0, 64.0, 154.0).get();
    let anchor = match c.radio("Anchor", &["Start", "Center", "End"], 1).get() {
        0 => ArcAnchor::Start,
        2 => ArcAnchor::End,
        _ => ArcAnchor::Center,
    };
    let facing = match c.radio("Facing", &["Outward", "Inward"], 0).get() {
        1 => ArcTextFacing::Inward,
        _ => ArcTextFacing::Outward,
    };

    let bg = Color::from_hex(0x0B1016);
    let center = 190.0;
    let ring = Color::from_rgba(0x80, 0x88, 0x92, 0xA0);

    c.ui.header(
        "Curved Text",
        "Inspect angle, radius, anchor, and facing on the reference circle.",
    );
    c.ui.div(
        (380, 380),
        canvas(
            props!(width: 380.0, height: 380.0, background: bg),
            [
                Draw::circle(center, center, radius + 1.0, ring),
                Draw::circle(center, center, radius - 1.0, bg),
                Draw::circle(center, center, 3.0, GRAY_50),
                Draw::curved_text(
                    center,
                    center,
                    radius,
                    angle,
                    anchor,
                    facing,
                    text.get(),
                    style!(
                        size: 28,
                        color: WHITE,
                        weight: FontWeight::BOLD,
                        family: FontFamily::DeckSans,
                    ),
                ),
            ],
        ),
    );
}

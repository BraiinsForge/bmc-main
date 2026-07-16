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

story_meta! { title: "Scroll" }

#[story(default)]
fn examples(c: &mut StoryCtx) {
    c.ui.header("Vertical scroll (200px viewport)", "");
    c.ui.div(
        (300, 200),
        scroll(
            "v_scroll",
            props!(background: GRAY_90),
            (0..20).map(|i| {
                text(
                    format!("Item {i}"),
                    style!(size: 14, color: GRAY_10, padding: 8),
                )
            }),
        ),
    );

    c.ui.header("Scroll with mixed content", "");
    c.ui.div(
        (400, 200),
        scroll(
            "mixed_scroll",
            props!(background: GRAY_90),
            [
                text(
                    "Header",
                    style!(size: 18, weight: FontWeight::BOLD, color: WHITE, padding: 8),
                ),
                col(
                    props!(gap: 4, padding: 8),
                    (0..15).map(|i| {
                        row(
                            props!(gap: 8, cross_align: CrossAlign::Center),
                            [
                                canvas(
                                    props!(width: 8, height: 8),
                                    [Draw::circle(
                                        4.0,
                                        4.0,
                                        4.0,
                                        if i % 2 == 0 { GREEN_50 } else { BLUE_50 },
                                    )],
                                ),
                                text(format!("List item {i}"), style!(size: 14, color: GRAY_10)),
                            ],
                        )
                    }),
                ),
            ],
        ),
    );
}

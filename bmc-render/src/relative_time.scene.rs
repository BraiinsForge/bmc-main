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

scene_meta! { title: "Components / Typography / Relative Time" }

#[scene(default)]
fn relative_time(ctx: &mut SceneCtx, ui: &mut Ui) {
    let age = ctx.slider("Age (s)", 90.0, 0.0, 200_000.0, 1.0);
    let future = ctx.toggle("Countdown (in …)", false);

    let length = match ctx.radio("Length", &["Short", "Long"], 0) {
        1 => RelTimeLength::Long,
        _ => RelTimeLength::Short,
    };

    let segments = match ctx.radio("Segments", &["Single", "Double"], 0) {
        1 => RelTimeSegments::Double,
        _ => RelTimeSegments::Single,
    };

    let secs = age as i64;

    // The gallery clock is 0, so the anchor is placed `secs` on either side of it.
    let anchor = SystemTime {
        unix_secs: if future { secs } else { -secs },
    };
    let format = RelTimeFormat { length, segments };

    ctx.node_stage(ui, (240_u32, 120_u32), || {
        center(
            props!(flex: 1.0),
            [relative_time_live(
                anchor,
                format,
                RelTimeClamp::Auto,
                TextStyle {
                    size: 24,
                    color: ORANGE_40,
                    ..Default::default()
                },
            )],
        )
    });
}

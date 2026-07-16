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

story_meta! { title: "NumberInput" }

#[story(default)]
fn examples(ctx: &mut StoryCtx) -> Node {
    let value = ctx.slider("Value", 25.0, 0.0, 100.0);
    let key = ctx.action("Change");
    ctx.bind(&key, "_plus", value.nudge(1.0));
    ctx.bind(&key, "_minus", value.nudge(-1.0));

    col(
        props!(padding: 16, gap: 24),
        [
            number_input!(&key, &value, label: "Temperature", suffix: "°C", min: 0, max: 100),
            number_input!(&key, &value, label: "Disabled", suffix: "min", min: 1, max: 60, disabled: true),
            number_input!(&key, &value, label: "Normal", suffix: "V", min: 0, max: 100),
            number_input!(&key, &value, label: "Warning", suffix: "W", min: 0, max: 100, warning: "Value is high"),
            number_input!(&key, &value, label: "Error", suffix: "%", min: 0, max: 100, error: "Exceeds maximum"),
        ],
    )
}

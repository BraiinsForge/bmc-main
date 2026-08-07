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

scene_meta! { title: "Components / Controls / NumberInput" }

/// Every input on the stage shares one key, so any pair of steppers drives the
/// same knob; the widget suffixes it per button.
const KEY: &str = "number";

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    let value = ctx.slider("Value", 25.0, 0.0, 100.0, 1.0);

    let fired = ctx.node_stage_input(ui,Page, || {
        col(
            props!(padding: 16, gap: 24),
            [
                number_input!(KEY, value as i32, label: "Temperature", suffix: "°C", min: 0, max: 100),
                number_input!(KEY, value as i32, label: "Disabled", suffix: "min", min: 1, max: 60, disabled: true),
                number_input!(KEY, value as i32, label: "Normal", suffix: "V", min: 0, max: 100),
                number_input!(KEY, value as i32, label: "Warning", suffix: "W", min: 0, max: 100, warning: "Value is high"),
                number_input!(KEY, value as i32, label: "Error", suffix: "%", min: 0, max: 100, error: "Exceeds maximum"),
            ],
        )
    });

    // The steppers move the knob that feeds them, so the controls panel and the
    // rendered value stay the same number.
    for (sub, step) in [("_plus", 1.0), ("_minus", -1.0)] {
        if fired.clicked(&format!("{KEY}{sub}")) {
            ctx.set_slider("Value", value + step);
            action(format!("Change {step:+}"));
        }
    }
}

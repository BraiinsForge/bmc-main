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

static SUZANNE: Mesh = include_mesh!("bmc-render/assets/suzanne.glb");
static WATER_BOTTLE: Mesh = include_mesh!("bmc-render/assets/water_bottle.glb");

story_meta! { title: "Canvas/Mesh" }

#[story(default)]
fn meshes(c: &mut StoryCtx) {
    c.group("Orientation");
    // Pitch+yaw collapsed into one pad. Direct mapping (no Y inversion):
    // because the pad is an absolute-position control, screen-Y matching
    // axis-Y reads more naturally than a camera-style invert. Roll stays
    // as a single 1-D slider — orthogonal to the pad.
    let pitch_yaw = c.pad2d(
        "Pitch / Yaw",
        Pad2DSpec {
            x: 30.0,
            y: 0.0,
            range_x: -180.0..=180.0,
            range_y: -180.0..=180.0,
            invert_y: false,
        },
    );
    let roll = c.slider("Roll", 0.0, -180.0, 180.0);
    c.group("Camera");
    let fov = c.slider("FOV", 35.0, 10.0, 120.0);
    let distance = c.slider("Distance", 3.0, 0.5, 10.0);
    // `Scale` only affects the water bottle (its source glTF is tiny);
    // Suzanne ships in a unit cube and renders well at scale 1.0.
    let bottle_scale = c.slider("Scale (water bottle)", 3.5, 0.5, 20.0);
    c.group("Lighting");
    let light_dir = c.pad2d(
        "Light direction",
        Pad2DSpec {
            x: -30.0,
            y: 45.0,
            range_x: -180.0..=180.0,
            range_y: -90.0..=90.0,
            invert_y: true,
        },
    );
    let ambient = c.slider("Ambient", 0.3, 0.0, 1.0);
    let specular = c.slider("Specular", 0.45, 0.0, 1.0);

    let orientation = Orientation::from_euler(pitch_yaw.y(), pitch_yaw.x(), roll.get());
    let light = Some(LightAngles {
        pitch: light_dir.y(),
        yaw: light_dir.x(),
    });
    let shared = MeshView {
        fov: fov.get(),
        distance: distance.get(),
        orientation,
        light,
        ambient: ambient.get(),
        specular: specular.get(),
        ..Default::default()
    };

    c.ui.header("Suzanne", "textured monkey head");
    c.ui.div(
        (320, 320),
        canvas(
            props!(width: 320, height: 320),
            [Draw::mesh(
                0.0,
                0.0,
                320.0,
                320.0,
                &SUZANNE,
                MeshView { ..shared },
            )],
        ),
    );

    c.ui.header(
        "Water Bottle",
        "Khronos glTF sample (CC0) — albedo + normal map",
    );
    c.ui.div(
        (320, 320),
        canvas(
            props!(width: 320, height: 320),
            [Draw::mesh(
                0.0,
                0.0,
                320.0,
                320.0,
                &WATER_BOTTLE,
                MeshView {
                    scale: bottle_scale.get(),
                    ..shared
                },
            )],
        ),
    );
}

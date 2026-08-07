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

static SUZANNE: Mesh = include_mesh!("bmc-render/assets/suzanne.glb");
static WATER_BOTTLE: Mesh = include_mesh!("bmc-render/assets/water_bottle.glb");

scene_meta! { title: "Components / Canvas / Mesh" }

#[scene(default)]
fn meshes(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Orientation");
    // Pitch+yaw collapsed into one pad. Direct mapping (no Y inversion):
    // because the pad is an absolute-position control, screen-Y matching
    // axis-Y reads more naturally than a camera-style invert. Roll stays
    // as a single 1-D slider — orthogonal to the pad.
    let Pad2D { x: yaw, y: pitch } = ctx.pad2d(
        "Pitch / Yaw",
        Pad2DSpec {
            default_x: 30.0,
            default_y: 0.0,
            min_x: -180.0,
            max_x: 180.0,
            min_y: -180.0,
            max_y: 180.0,
            invert_y: false,
        },
    );
    let roll = ctx.slider("Roll", 0.0, -180.0, 180.0, 1.0);
    ctx.group("Camera");
    let fov = ctx.slider("FOV", 35.0, 10.0, 120.0, 1.0);
    let distance = ctx.slider("Distance", 3.0, 0.5, 10.0, 0.1);
    // `Scale` only affects the water bottle (its source glTF is tiny);
    // Suzanne ships in a unit cube and renders well at scale 1.0.
    let bottle_scale = ctx.slider("Scale (water bottle)", 3.5, 0.5, 20.0, 0.1);
    ctx.group("Lighting");
    let Pad2D {
        x: light_yaw,
        y: light_pitch,
    } = ctx.pad2d(
        "Light direction",
        Pad2DSpec {
            default_x: -30.0,
            default_y: 45.0,
            min_x: -180.0,
            max_x: 180.0,
            min_y: -90.0,
            max_y: 90.0,
            invert_y: true,
        },
    );
    let ambient = ctx.slider("Ambient", 0.3, 0.0, 1.0, 0.01);
    let specular = ctx.slider("Specular", 0.45, 0.0, 1.0, 0.01);

    let orientation = Orientation::from_euler(pitch, yaw, roll);
    let light = Some(LightAngles {
        pitch: light_pitch,
        yaw: light_yaw,
    });
    let shared = MeshView {
        fov,
        distance,
        orientation,
        light,
        ambient,
        specular,
        ..Default::default()
    };

    ui.heading("Suzanne");
    ui.label("textured monkey head");
    ctx.node_stage(ui, (320_u32, 320_u32), || {
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
        )
    });

    ui.heading("Water Bottle");
    ui.label("Khronos glTF sample (CC0) — albedo + normal map");
    ctx.node_stage(ui, (320_u32, 320_u32), || {
        canvas(
            props!(width: 320, height: 320),
            [Draw::mesh(
                0.0,
                0.0,
                320.0,
                320.0,
                &WATER_BOTTLE,
                MeshView {
                    scale: bottle_scale,
                    ..shared
                },
            )],
        )
    });
}

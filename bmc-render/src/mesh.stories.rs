// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

static SUZANNE: Mesh = include_mesh!("bmc-wasm-runtime/examples/mesh-demo/assets/suzanne.glb");
static WATER_BOTTLE: Mesh = include_mesh!("bmc-render/assets/water_bottle.glb");

story_meta! { title: "Canvas/Mesh" }

#[story(default)]
fn meshes(c: &mut StoryCtx) {
    c.group("Orientation");
    let pitch = c.slider("Pitch", 0.0, -180.0, 180.0);
    let yaw = c.slider("Yaw", 30.0, -180.0, 180.0);
    let roll = c.slider("Roll", 0.0, -180.0, 180.0);
    c.group("Camera");
    let fov = c.slider("FOV", 35.0, 10.0, 120.0);
    let distance = c.slider("Distance", 3.0, 0.5, 10.0);
    // `Scale` only affects the water bottle (its source glTF is tiny);
    // Suzanne ships in a unit cube and renders well at scale 1.0.
    let bottle_scale = c.slider("Scale (water bottle)", 3.5, 0.5, 20.0);
    c.group("Lighting");
    let light_pitch = c.slider("Light pitch", 45.0, -90.0, 90.0);
    let light_yaw = c.slider("Light yaw", -30.0, -180.0, 180.0);
    let ambient = c.slider("Ambient", 0.3, 0.0, 1.0);
    let specular = c.slider("Specular", 0.45, 0.0, 1.0);

    let orientation = Orientation::from_euler(pitch.get(), yaw.get(), roll.get());
    let light = Some(LightAngles {
        pitch: light_pitch.get(),
        yaw: light_yaw.get(),
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

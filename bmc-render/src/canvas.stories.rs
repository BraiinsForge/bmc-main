// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

const STORYBOOK_BITMAP: Bitmap = include_bitmap!("bmc-render/assets/test_bitmap.png");

story_meta! { title: "Canvas/Shapes" }

#[story(default)]
fn shapes(c: &mut StoryCtx) {
    c.ui.div(
        (300, 200),
        canvas(
            props!(width: 300, height: 200),
            [
                Draw::rect(10.0, 10.0, 80.0, 60.0, VIOLET_60),
                Draw::circle(180.0, 50.0, 30.0, RED_50),
                Draw::rect(10.0, 90.0, 280.0, 4.0, GRAY_60),
                Draw::circle(50.0, 150.0, 20.0, GREEN_40),
                Draw::circle(110.0, 150.0, 20.0, YELLOW_30),
                Draw::circle(170.0, 150.0, 20.0, BLUE_50),
            ],
        ),
    );
    c.ui.div(
        (300, 150),
        canvas(
            props!(width: 300, height: 150),
            [
                path!(
                    vec![
                        (10.0, 130.0),
                        (60.0, 20.0),
                        (110.0, 130.0),
                        (160.0, 20.0),
                        (210.0, 130.0),
                        (260.0, 20.0),
                    ],
                    stroke: 2.0,
                    color: VIOLET_40
                ),
                path!(
                    vec![
                        (10.0, 130.0),
                        (60.0, 20.0),
                        (110.0, 130.0),
                        (160.0, 20.0),
                        (210.0, 130.0),
                        (260.0, 20.0),
                    ],
                    stroke: 2.0,
                    color: GREEN_40,
                    smooth
                ),
            ],
        ),
    );
}

#[story]
fn bitmap(c: &mut StoryCtx) {
    c.ui.header(
        "Draw::Bitmap",
        "Compile-time-embedded raster, drawn at three sizes",
    );
    c.ui.div(
        (300, 100),
        canvas(
            props!(width: 300, height: 100),
            [
                Draw::bitmap(8.0, 18.0, 64.0, 64.0, &STORYBOOK_BITMAP),
                Draw::bitmap(96.0, 26.0, 48.0, 48.0, &STORYBOOK_BITMAP),
                Draw::bitmap(168.0, 34.0, 32.0, 32.0, &STORYBOOK_BITMAP),
            ],
        ),
    );
}

#[story]
fn sphere(c: &mut StoryCtx) {
    c.ui.header(
        "Draw::Sphere",
        "Equirectangular bitmap projected onto a sphere with optional atmosphere and directional light",
    );

    // 2-axis pad bundles longitude+latitude so the user drags one control
    // for the sphere centre instead of two orthogonal sliders. X = lon,
    // Y = lat (lat positive up, matching the map convention).
    let centre = c.pad2d(
        "Sphere centre",
        Pad2DSpec {
            x: -45.0,
            y: 20.0,
            range_x: -180.0..=180.0,
            range_y: -90.0..=90.0,
            invert_y: false,
        },
    );
    // `zoom` is camera-distance-with-auto-fit-FOV (Google-Earth altitude
    // semantics): the sphere's apparent size on screen stays roughly
    // constant; what changes is texture detail and the visible region.
    // Below ~1.5 the camera sits on the surface and the projection
    // degenerates into a flat-looking texture region — clamp the lower
    // bound to keep the demo recognisable as a sphere.
    let zoom = c.slider("Zoom", 3.0, 1.5, 6.0);
    let atmosphere = c.toggle("Atmosphere", true);
    let light = c.pad2d(
        "Light direction",
        Pad2DSpec {
            x: -60.0,
            y: 30.0,
            range_x: -180.0..=180.0,
            range_y: -90.0..=90.0,
            invert_y: true,
        },
    );

    // Render at 480×480 so the silhouette aliasing in the sphere shader
    // (a hard `disc < 0` boundary) is below perceptual threshold — same
    // shader is used for ISS at 560×480 and looks fine. A proper shader
    // anti-alias is tracked separately; would benefit small-sphere uses
    // but needs Vivante GC400 measurement before landing.
    c.ui.div(
        (480, 480),
        canvas(
            props!(width: 480, height: 480),
            [Draw::sphere(
                40.0,
                40.0,
                400.0,
                400.0,
                &STORYBOOK_BITMAP,
                centre.y(),
                centre.x(),
                zoom.get(),
                Some((light.y(), light.x())),
                atmosphere.get(),
            )],
        ),
    );
}

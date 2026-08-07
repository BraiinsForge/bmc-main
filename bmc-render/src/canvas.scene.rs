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

const TEST_BITMAP: Bitmap = include_bitmap!("bmc-render/assets/test_bitmap.png");

scene_meta! { title: "Components / Canvas / Shapes" }

#[scene(default)]
fn shapes(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.node_stage(ui, (300_u32, 200_u32), || {
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
        )
    });
    ctx.node_stage(ui, (300_u32, 150_u32), || {
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
        )
    });
}

#[scene]
fn bitmap(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Draw::Bitmap");
    ui.label("Compile-time-embedded raster, drawn at three sizes");
    ctx.node_stage(ui, (300_u32, 100_u32), || {
        canvas(
            props!(width: 300, height: 100),
            [
                Draw::bitmap(8.0, 18.0, 64.0, 64.0, &TEST_BITMAP),
                Draw::bitmap(96.0, 26.0, 48.0, 48.0, &TEST_BITMAP),
                Draw::bitmap(168.0, 34.0, 32.0, 32.0, &TEST_BITMAP),
            ],
        )
    });
}

#[scene]
fn sphere(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Draw::Sphere");
    ui.label(
        "Equirectangular bitmap projected onto a sphere with optional atmosphere and directional light",
    );

    // 2-axis pad bundles longitude+latitude so the user drags one control
    // for the sphere centre instead of two orthogonal sliders. X = lon,
    // Y = lat (lat positive up, matching the map convention).
    let Pad2D { x: lon, y: lat } = ctx.pad2d(
        "Sphere centre",
        Pad2DSpec {
            default_x: -45.0,
            default_y: 20.0,
            min_x: -180.0,
            max_x: 180.0,
            min_y: -90.0,
            max_y: 90.0,
            invert_y: false,
        },
    );
    // `zoom` is camera-distance-with-auto-fit-FOV (Google-Earth altitude
    // semantics): the sphere's apparent size on screen stays roughly
    // constant; what changes is texture detail and the visible region.
    // Below ~1.5 the camera sits on the surface and the projection
    // degenerates into a flat-looking texture region — clamp the lower
    // bound to keep the demo recognisable as a sphere.
    let zoom = ctx.slider("Zoom", 3.0, 1.5, 6.0, 0.1);
    let atmosphere = ctx.toggle("Atmosphere", true);
    let Pad2D {
        x: light_lon,
        y: light_lat,
    } = ctx.pad2d(
        "Light direction",
        Pad2DSpec {
            default_x: -60.0,
            default_y: 30.0,
            min_x: -180.0,
            max_x: 180.0,
            min_y: -90.0,
            max_y: 90.0,
            invert_y: true,
        },
    );

    // Render at 480×480 so the silhouette aliasing in the sphere shader
    // (a hard `disc < 0` boundary) is below perceptual threshold — same
    // shader is used for ISS at 560×480 and looks fine. A proper shader
    // anti-alias is tracked separately; would benefit small-sphere uses
    // but needs Vivante GC400 measurement before landing.
    ctx.node_stage(ui, (480_u32, 480_u32), || {
        canvas(
            props!(width: 480, height: 480),
            [Draw::sphere(
                40.0,
                40.0,
                400.0,
                400.0,
                &TEST_BITMAP,
                lat,
                lon,
                zoom,
                Some((light_lat, light_lon)),
                atmosphere,
            )],
        )
    });
}

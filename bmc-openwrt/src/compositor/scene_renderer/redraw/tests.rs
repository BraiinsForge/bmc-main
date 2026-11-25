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

use bmc_platform::{DisplayTransform, FrameRedraw};
use smithay::utils::{Buffer as BufferCoord, Logical, Physical, Rectangle, Size, Transform};

use super::super::{
    CombinedSceneGrid, RenderableItem, RowOrder, bottom_up_transform, place_widget,
    scanout_transform,
};
use super::{
    FrameChange, FrameExtras, FrameLayout, LastFrame, Redraw, buffer_damage_on_output,
    damage_within, disjoint, plan_frame, widget_change,
};
use crate::compositor::output_damage::{OutputDamage, WidgetDamage};

fn tex() -> Size<i32, BufferCoord> {
    Size::new(1280, 480)
}

/// The panel's physical size for a profile, as `place_widget` takes it.
fn panel(profile: DisplayTransform) -> Size<i32, Physical> {
    match profile {
        DisplayTransform::Deg0 => Size::new(tex().w, tex().h),
        DisplayTransform::Deg90 | DisplayTransform::Deg270 => Size::new(tex().h, tex().w),
    }
}

/// Where damage at logical `region` inside a widget at logical `widget_x`
/// must land on the Deck: where `place_widget` puts that rectangle.
/// The buffer holds it at rows `tex_h - y - h`, since GL writes the bottom row first.
fn deck_region_lands_where_placed(widget_x: i32, widget_w: i32, region: Rectangle<i32, Logical>) {
    let (x, y, w, h) = (region.loc.x, region.loc.y, region.size.w, region.size.h);
    let panel_size = panel(DisplayTransform::Deg270);
    let dst = place_widget(
        widget_x,
        0,
        widget_w,
        tex().h,
        panel_size.w,
        panel_size.h,
        DisplayTransform::Deg270,
    );
    let buffer = Rectangle::<i32, BufferCoord>::new((x, tex().h - y - h).into(), (w, h).into());

    assert_eq!(
        buffer_damage_on_output(
            buffer,
            Size::from((widget_w, tex().h)),
            bottom_up_transform(DisplayTransform::Deg270),
            dst,
        ),
        Some(place_widget(
            widget_x + x,
            y,
            w,
            h,
            panel_size.w,
            panel_size.h,
            DisplayTransform::Deg270
        )),
        "region {region:?} of the widget at x {widget_x}"
    );
}

#[test]
fn deck_damage_near_a_fullscreen_widgets_top_left_lands_where_that_corner_is_drawn() {
    deck_region_lands_where_placed(
        0,
        tex().w,
        Rectangle::new((10, 20).into(), (100, 40).into()),
    );
}

#[test]
fn deck_damage_in_the_right_half_widget_lands_where_that_widget_draws_it() {
    deck_region_lands_where_placed(642, 638, Rectangle::new((10, 20).into(), (100, 40).into()));
}

#[test]
fn untransformed_buffer_damage_only_moves_to_the_destination() {
    let region = Rectangle::<i32, BufferCoord>::new((100, 40).into(), (20, 10).into());
    let dst = Rectangle::new((7, 11).into(), (tex().w, tex().h).into());

    assert_eq!(
        buffer_damage_on_output(region, tex(), Transform::Normal, dst),
        Some(Rectangle::new((107, 51).into(), (20, 10).into()))
    );
}

#[test]
fn the_whole_buffer_covers_the_whole_destination_under_every_profile_transform() {
    let whole = Rectangle::<i32, BufferCoord>::from_size(tex());
    for profile in [
        DisplayTransform::Deg0,
        DisplayTransform::Deg90,
        DisplayTransform::Deg270,
    ] {
        let panel_size = panel(profile);
        let dst = place_widget(0, 0, tex().w, tex().h, panel_size.w, panel_size.h, profile);
        for transform in [scanout_transform(profile), bottom_up_transform(profile)] {
            assert_eq!(
                buffer_damage_on_output(whole, tex(), transform, dst),
                Some(dst),
                "{profile:?} drawn with {transform:?}"
            );
        }
    }
}

fn rect(x: i32, w: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, 0).into(), (w, 10).into())
}

fn layout_with_clock_at(dst: Rectangle<i32, Physical>) -> FrameLayout {
    FrameLayout::of(&[item("clock", dst)], &[])
}

fn known(change: Rectangle<i32, Physical>) -> LastFrame {
    LastFrame {
        layout: layout_with_clock_at(rect(0, 100)),
        change: FrameChange::Known(vec![change]),
    }
}

fn item(instance_id: &str, dst: Rectangle<i32, Physical>) -> RenderableItem {
    RenderableItem {
        buffer_id: smithay::reexports::wayland_server::backend::ObjectId::null(),
        instance_id: instance_id.to_owned(),
        dst,
        alpha: 1.0,
        row_order: RowOrder::BottomUp,
    }
}

#[test]
fn a_different_opacity_row_order_or_grid_changes_the_layout() {
    let base = || item("clock", rect(0, 100));
    let fading = RenderableItem {
        alpha: 0.5,
        ..base()
    };
    let top_down = RenderableItem {
        row_order: RowOrder::TopDown,
        ..base()
    };
    let grid_at = |x_offset| CombinedSceneGrid {
        x_offset,
        alpha: 1.0,
    };
    let layout = FrameLayout::of(&[base()], &[grid_at(0)]);

    for (other, what) in [
        (FrameLayout::of(&[fading], &[grid_at(0)]), "opacity"),
        (FrameLayout::of(&[top_down], &[grid_at(0)]), "row order"),
        (FrameLayout::of(&[base()], &[]), "separator grid"),
        (FrameLayout::of(&[base()], &[grid_at(40)]), "grid offset"),
    ] {
        assert_ne!(layout, other, "a different {what} draws other pixels");
    }
}

fn owed(instance_id: &str, damage: WidgetDamage) -> OutputDamage {
    let mut owed = OutputDamage::default();
    owed.mark_widget(&instance_id.to_owned(), damage);
    owed
}

#[test]
fn a_whole_widget_changes_its_destination() {
    let dst = rect(200, 100);

    assert_eq!(
        widget_change(
            |_| Some(Size::from((100, 10))),
            DisplayTransform::Deg0,
            &owed("clock", WidgetDamage::Whole),
            &[item("clock", dst)],
        ),
        vec![dst]
    );
}

#[test]
fn a_widget_owing_nothing_or_without_a_texture_changes_nothing() {
    let damage = owed("clock", WidgetDamage::Whole);
    let owing_nothing = widget_change(
        |_| Some(Size::from((100, 10))),
        DisplayTransform::Deg0,
        &damage,
        &[item("weather", rect(0, 100))],
    );
    let without_texture = widget_change(
        |_| None,
        DisplayTransform::Deg0,
        &damage,
        &[item("clock", rect(0, 100))],
    );

    assert_eq!((owing_nothing, without_texture), (Vec::new(), Vec::new()));
}

#[test]
fn deck_region_damage_lands_through_the_row_flipped_transform() {
    let panel_size = panel(DisplayTransform::Deg270);
    let dst = place_widget(
        0,
        0,
        tex().w,
        tex().h,
        panel_size.w,
        panel_size.h,
        DisplayTransform::Deg270,
    );
    let region =
        Rectangle::<i32, BufferCoord>::new((10, tex().h - 20 - 40).into(), (100, 40).into());

    assert_eq!(
        widget_change(
            |_| Some(tex()),
            DisplayTransform::Deg270,
            &owed("clock", WidgetDamage::Regions(vec![region])),
            &[item("clock", dst)],
        ),
        vec![place_widget(
            10,
            20,
            100,
            40,
            panel_size.w,
            panel_size.h,
            DisplayTransform::Deg270
        )],
        "a bottom-up buffer is drawn with the profile's transform row-flipped"
    );
}

#[test]
fn a_draws_damage_is_the_redraw_inside_its_destination_relative_to_it() {
    let dst = Rectangle::new((100, 50).into(), (40, 40).into());
    let straddling = Rectangle::new((120, 30).into(), (40, 40).into());

    assert_eq!(
        damage_within(dst, &Redraw::Only(vec![straddling])),
        vec![Rectangle::new((20, 0).into(), (20, 20).into())]
    );
}

#[test]
fn a_redraw_elsewhere_leaves_a_draw_no_damage() {
    let dst = Rectangle::new((100, 50).into(), (40, 40).into());

    assert!(damage_within(dst, &Redraw::Only(vec![rect(0, 10)])).is_empty());
}

#[test]
fn a_full_redraw_damages_the_whole_destination() {
    let dst = Rectangle::new((100, 50).into(), (40, 40).into());

    assert_eq!(
        damage_within(dst, &Redraw::Everything),
        vec![Rectangle::from_size(dst.size)]
    );
}

#[test]
fn extreme_client_damage_is_clipped_before_it_is_mapped() {
    let dst = Rectangle::new((7, 11).into(), (tex().w, tex().h).into());
    let everything =
        Rectangle::<i32, BufferCoord>::new((-5, -5).into(), (i32::MAX, i32::MAX).into());
    let far_away = Rectangle::<i32, BufferCoord>::new((i32::MAX - 5, 0).into(), (10, 10).into());

    assert_eq!(
        (
            buffer_damage_on_output(everything, tex(), Transform::Flipped180, dst),
            buffer_damage_on_output(far_away, tex(), Transform::Flipped180, dst),
        ),
        (Some(dst), None)
    );
}

#[test]
fn widgets_drawn_in_another_order_keep_the_layout() {
    let (left, right) = (rect(0, 100), rect(200, 100));

    assert_eq!(
        FrameLayout::of(&[item("a", left), item("b", right)], &[]),
        FrameLayout::of(&[item("b", right), item("a", left)], &[]),
        "each commit moves a widget's buffer to the end of the draw order"
    );
}

/// A frame with no extras on a board that redraws only what changed.
fn plan(
    damage: &OutputDamage,
    last: Option<LastFrame>,
    layout: FrameLayout,
    change: Vec<Rectangle<i32, Physical>>,
) -> (Redraw, Option<LastFrame>) {
    plan_frame(
        FrameRedraw::Changed,
        FrameExtras::default(),
        damage,
        last,
        || layout,
        || change,
    )
}

#[test]
fn every_extra_redraws_everything_and_leaves_nothing_to_build_on() {
    let extras = [
        FrameExtras {
            transition: true,
            ..FrameExtras::default()
        },
        FrameExtras {
            drag: true,
            ..FrameExtras::default()
        },
        FrameExtras {
            layers: true,
            ..FrameExtras::default()
        },
        FrameExtras {
            logo: true,
            ..FrameExtras::default()
        },
        FrameExtras {
            probe: true,
            ..FrameExtras::default()
        },
    ];
    for extras in extras {
        assert_eq!(
            plan_frame(
                FrameRedraw::Changed,
                extras,
                &OutputDamage::default(),
                Some(known(rect(0, 5))),
                || layout_with_clock_at(rect(0, 100)),
                || vec![rect(0, 5)],
            ),
            (Redraw::Everything, None),
            "{extras:?}"
        );
    }
}

#[test]
fn a_board_that_redraws_whole_frames_never_lays_one_out() {
    assert_eq!(
        plan_frame(
            FrameRedraw::Whole,
            FrameExtras::default(),
            &OutputDamage::default(),
            Some(known(rect(0, 5))),
            || panic!("a whole-frame board planned a layout"),
            || vec![rect(0, 5)],
        ),
        (Redraw::Everything, None)
    );
}

#[test]
fn the_first_widget_frames_after_a_structural_one_redraw_in_full_until_both_changes_are_known() {
    let layout = layout_with_clock_at(rect(0, 100));
    let damage = OutputDamage::default();

    let (first, after_first) = plan(&damage, None, layout.clone(), vec![rect(0, 100)]);
    let (second, after_second) = plan(&damage, after_first, layout.clone(), vec![rect(10, 5)]);
    let (third, _) = plan(&damage, after_second, layout, vec![rect(20, 5)]);

    assert_eq!(
        (first, second, third),
        (
            Redraw::Everything,
            Redraw::Everything,
            Redraw::Only(vec![rect(20, 5), rect(10, 5)])
        ),
        "the logo the structural frame drew is in no widget's damage, \
         so only the third frame may build on known changes"
    );
}

#[test]
fn a_widget_that_moved_or_resized_redraws_in_full() {
    for (dst, what) in [(rect(0, 80), "resized"), (rect(20, 100), "moved")] {
        let (redraw, next) = plan(
            &OutputDamage::default(),
            Some(known(rect(0, 5))),
            layout_with_clock_at(dst),
            vec![rect(0, 5)],
        );

        assert_eq!(
            (redraw, next.map(|next| next.change)),
            (Redraw::Everything, Some(FrameChange::Unknown)),
            "the area the {what} clock's old destination covered is in no damage"
        );
    }
}

#[test]
fn full_damage_redraws_everything_and_leaves_its_change_unknown() {
    let layout = layout_with_clock_at(rect(0, 100));

    let (redraw, next) = plan(
        &OutputDamage::Full,
        Some(known(rect(0, 5))),
        layout,
        vec![rect(0, 5)],
    );

    assert_eq!(
        (redraw, next.map(|next| next.change)),
        (Redraw::Everything, Some(FrameChange::Unknown))
    );
}

#[test]
fn overlapping_rectangles_are_redrawn_once() {
    let first = Rectangle::<i32, Physical>::new((0, 0).into(), (10, 10).into());
    let second = Rectangle::new((5, 0).into(), (10, 10).into());
    let parts = disjoint([&first, &second]);

    let area: i32 = parts.iter().map(|r| r.size.w * r.size.h).sum();
    assert_eq!(
        area, 150,
        "two 100 px squares overlapping by 50 px: {parts:?}"
    );
}

/// The output pixels `render_texture_from_to` lights when it draws a `tex_size` texture,
/// lit only under buffer `region`, into `dst` with `transform`; `None` without headless EGL.
fn drawn_region(
    region: Rectangle<i32, BufferCoord>,
    tex_size: Size<i32, BufferCoord>,
    transform: Transform,
    output: Size<i32, Physical>,
    dst: Rectangle<i32, Physical>,
) -> Option<Rectangle<i32, Physical>> {
    use smithay::backend::allocator::Fourcc;
    use smithay::backend::renderer::{
        Bind, Color32F, ExportMem, Frame as _, ImportMem, Offscreen, Renderer as _,
        gles::GlesTexture,
    };

    let mut renderer = crate::compositor::render::headless_egl::headless_renderer()?;
    let index = |v: i32| usize::try_from(v).expect("BUG: test geometry is non-negative");
    let mut texels = vec![0_u8; index(tex_size.w) * index(tex_size.h) * 4];
    for y in region.loc.y..region.loc.y + region.size.h {
        for x in region.loc.x..region.loc.x + region.size.w {
            let at = (index(y) * index(tex_size.w) + index(x)) * 4;
            texels[at..at + 4].copy_from_slice(&[0xFF; 4]);
        }
    }
    let texture = renderer
        .import_memory(
            &texels,
            Fourcc::Abgr8888,
            Size::from((tex_size.w, tex_size.h)),
            false,
        )
        .expect("BUG: llvmpipe imports RGBA8 textures");
    let mut target: GlesTexture = renderer
        .create_buffer(Fourcc::Abgr8888, Size::from((output.w, output.h)))
        .expect("BUG: llvmpipe renders into RGBA8 textures");
    {
        let mut framebuffer = renderer.bind(&mut target).expect("BUG: a new target binds");
        let mut frame = renderer
            .render(&mut framebuffer, output, Transform::Normal)
            .expect("BUG: a frame begins on a bound target");
        frame
            .clear(
                Color32F::new(0.0, 0.0, 0.0, 1.0),
                &[Rectangle::from_size(output)],
            )
            .expect("BUG: llvmpipe clears");
        frame
            .render_texture_from_to(
                &texture,
                Rectangle::from_size(Size::from((f64::from(tex_size.w), f64::from(tex_size.h)))),
                dst,
                &[Rectangle::from_size(dst.size)],
                &[],
                transform,
                1.0,
                None,
                &[],
            )
            .expect("BUG: llvmpipe draws a texture");
        let _sync = frame.finish().expect("BUG: llvmpipe finishes a frame");
    }
    let framebuffer = renderer
        .bind(&mut target)
        .expect("BUG: the target was just rendered into");
    let mapping = renderer
        .copy_framebuffer(
            &framebuffer,
            Rectangle::from_size(Size::from((output.w, output.h))),
            Fourcc::Abgr8888,
        )
        .expect("BUG: llvmpipe reads back RGBA8");
    let pixels = renderer
        .map_texture(&mapping)
        .expect("BUG: a finished copy maps");
    let coord = |v: usize| i32::try_from(v).expect("BUG: test geometry fits i32");
    let (pixels, _) = pixels.as_chunks::<4>();
    let lit: Vec<(i32, i32)> = pixels
        .iter()
        .enumerate()
        .filter(|(_, pixel)| pixel[0] > 0x80)
        .map(|(at, _)| {
            let width = index(output.w);
            (coord(at % width), coord(at.div_euclid(width)))
        })
        .collect();
    let (left, right) = (
        lit.iter().map(|p| p.0).min()?,
        lit.iter().map(|p| p.0).max()?,
    );
    let (top, bottom) = (
        lit.iter().map(|p| p.1).min()?,
        lit.iter().map(|p| p.1).max()?,
    );
    Some(Rectangle::new(
        (left, top).into(),
        (right - left + 1, bottom - top + 1).into(),
    ))
}

#[test]
fn region_damage_lands_where_its_texels_are_drawn() {
    let tex_size = Size::<i32, BufferCoord>::new(64, 32);
    let region = Rectangle::<i32, BufferCoord>::new((4, 2).into(), (10, 6).into());
    let output = Size::<i32, Physical>::new(100, 100);
    for profile in [
        DisplayTransform::Deg0,
        DisplayTransform::Deg90,
        DisplayTransform::Deg270,
    ] {
        for row_order in [RowOrder::TopDown, RowOrder::BottomUp] {
            let transform = row_order.scanout_transform(profile);
            let dst_size = match profile {
                DisplayTransform::Deg0 => Size::new(tex_size.w, tex_size.h),
                DisplayTransform::Deg90 | DisplayTransform::Deg270 => {
                    Size::new(tex_size.h, tex_size.w)
                }
            };
            let dst = Rectangle::new((7, 11).into(), dst_size);
            let Some(drawn) = drawn_region(region, tex_size, transform, output, dst) else {
                return;
            };

            assert_eq!(
                buffer_damage_on_output(region, tex_size, transform, dst),
                Some(drawn),
                "{profile:?} {row_order:?} ({transform:?})"
            );
        }
    }
}

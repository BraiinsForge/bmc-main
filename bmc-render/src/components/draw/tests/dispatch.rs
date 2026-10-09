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

use super::*;

struct RecordingAssetResolver {
    asset_events: Rc<RefCell<Vec<AssetEvent>>>,
}

impl RendererAssetResolver for RecordingAssetResolver {
    fn resolve_svg(&mut self, _renderer: &mut dyn Renderer, id: SvgId) -> bool {
        self.asset_events
            .borrow_mut()
            .push(AssetEvent::Resolve(Asset::Svg(id)));
        true
    }

    fn resolve_bitmap(&mut self, _renderer: &mut dyn Renderer, id: BitmapId) -> bool {
        self.asset_events
            .borrow_mut()
            .push(AssetEvent::Resolve(Asset::Bitmap(id)));
        true
    }

    fn resolve_mesh(&mut self, _renderer: &mut dyn Renderer, id: MeshId) -> bool {
        self.asset_events
            .borrow_mut()
            .push(AssetEvent::Resolve(Asset::Mesh(id)));
        true
    }
}

fn translated(translate_x: f32, translate_y: f32) -> DrawOffset {
    DrawOffset {
        translate_x,
        translate_y,
        ..DrawOffset::default()
    }
}

#[test]
fn qr_renders_a_code_that_scans_back_to_its_text() {
    // Decoded by an independent reader (rqrr), not our own code — so a
    // packing or transpose bug can't quietly roundtrip through itself.
    let text = "https://deck.local/setup?x=42";
    let size = 500.0;
    let mut rr = RecordingRenderer::default();
    render_qr(&mut rr, 0.0, 0.0, size, BLACK, WHITE, 4, text);

    // Replay the recorded rects onto a white canvas.
    let dim = size as u32;
    let mut img = image::GrayImage::from_pixel(dim, dim, image::Luma([255]));
    for &(x, y, w, h, color) in &rr.rects {
        let luma = if color.to_u32() == BLACK.to_u32() {
            0
        } else {
            255
        };
        let x0 = x.round().max(0.0) as u32;
        let y0 = y.round().max(0.0) as u32;
        let x1 = ((x + w).round() as u32).min(dim);
        let y1 = ((y + h).round() as u32).min(dim);
        for py in y0..y1 {
            for px in x0..x1 {
                img.put_pixel(px, py, image::Luma([luma]));
            }
        }
    }

    let mut prepared = rqrr::PreparedImage::prepare(img);
    let grids = prepared.detect_grids();
    assert_eq!(grids.len(), 1, "exactly one QR grid must be detected");
    let (_meta, decoded) = grids[0]
        .decode()
        .expect("BUG: the rendered grid must decode");
    assert_eq!(decoded, text, "scanned text must equal the encoded input");
}

#[test]
fn render_target_resolves_asset_ids_at_each_draw_including_shadow_content() {
    let svg = SvgId::from_wire(1).expect("BUG: fixture SVG ID must be non-zero");
    let bitmap = BitmapId::from_wire(2).expect("BUG: fixture bitmap ID must be non-zero");
    let mesh = MeshId::from_wire(3).expect("BUG: fixture mesh ID must be non-zero");
    let mut renderer = RecordingRenderer::default();
    let mut resolver = RecordingAssetResolver {
        asset_events: Rc::clone(&renderer.asset_events),
    };
    {
        let resolver = RefCell::new(&mut resolver as &mut dyn RendererAssetResolver);
        let mut target = RenderTarget::new(&mut renderer, Some(&resolver));
        target.drop_shadow(
            0.0,
            0.0,
            1,
            1,
            0.0,
            0.0,
            0.0,
            Color::default(),
            &mut |target| {
                target.draw_svg(0.0, 0.0, 1.0, 1.0, WHITE, svg, false, &[]);
            },
        );
        target.draw_bitmap(0.0, 0.0, 1.0, 1.0, bitmap);
        target.draw_nine_patch(0.0, 0.0, 1.0, 1.0, bitmap, 0, 0, 0, 0);
        target.draw_sphere(
            0.0,
            0.0,
            1.0,
            1.0,
            bitmap,
            0.0,
            0.0,
            1.0,
            f32::NAN,
            0.0,
            false,
        );
        target.draw_mesh(
            0.0,
            0.0,
            1.0,
            1.0,
            0,
            mesh,
            crate::gpu::mesh::ZERO_MESH_ARGS,
        );
    }

    let expected = [
        AssetEvent::Resolve(Asset::Svg(svg)),
        AssetEvent::Draw(Asset::Svg(svg)),
        AssetEvent::Resolve(Asset::Bitmap(bitmap)),
        AssetEvent::Draw(Asset::Bitmap(bitmap)),
        AssetEvent::Resolve(Asset::Bitmap(bitmap)),
        AssetEvent::Draw(Asset::Bitmap(bitmap)),
        AssetEvent::Resolve(Asset::Bitmap(bitmap)),
        AssetEvent::Draw(Asset::Bitmap(bitmap)),
        AssetEvent::Resolve(Asset::Mesh(mesh)),
        AssetEvent::Draw(Asset::Mesh(mesh)),
    ];
    assert_eq!(*renderer.asset_events.borrow(), expected);
}

#[test]
fn curved_text_dispatches_scaled_style_and_color_override() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
    let draw = DrawCommand::CurvedText {
        cx: 5.0,
        cy: 6.0,
        radius: 7.0,
        angle: 0.25,
        anchor: ArcAnchor::Center,
        facing: ArcTextFacing::Inward,
        text: "hashrate".to_owned(),
        style: TextStyle {
            size: 12,
            color: Color::from_rgb(1, 2, 3),
            ..Default::default()
        },
    };

    render_draw_inner_for_test(
        &mut renderer,
        &draw,
        10.0,
        20.0,
        100.0,
        80.0,
        translated(3.0, 4.0),
        0.0,
        2.0,
        0.5,
        0.0,
        Some(Color::from_rgba(10, 20, 30, 128)),
        &mut anim_ctx,
    );

    let expected_style = TextStyle {
        size: 24,
        color: Color::from_rgba(10, 20, 30, 64),
        ..Default::default()
    };
    let [
        RenderEvent::CurvedText {
            cx: 18.0,
            cy: 30.0,
            radius: 14.0,
            angle: 0.25,
            anchor: ArcAnchor::Center,
            facing: ArcTextFacing::Inward,
            text,
            style,
        },
    ] = &renderer.events[..]
    else {
        panic!("expected one curved text draw event");
    };
    assert_eq!(text, "hashrate");
    assert_eq!(style.size, expected_style.size);
    assert_eq!(style.color, expected_style.color);
}

#[test]
fn an_outer_translate_moves_a_centered_draw() {
    fn centre_of(draw: &DrawCommand) -> (f32, f32) {
        let mut renderer = RecordingRenderer::default();
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);

        render_draw_inner_for_test(
            &mut renderer,
            draw,
            0.0,
            0.0,
            100.0,
            80.0,
            DrawOffset::default(),
            0.0,
            1.0,
            1.0,
            0.0,
            None,
            &mut anim_ctx,
        );

        let [RenderEvent::CurvedText { cx, cy, .. }] = &renderer.events[..] else {
            panic!("BUG: expected one curved text draw event");
        };
        (*cx, *cy)
    }

    let centred = || DrawCommand::Centered {
        inner: Box::new(DrawCommand::CurvedText {
            cx: 0.0,
            cy: 0.0,
            radius: 7.0,
            angle: 0.0,
            anchor: ArcAnchor::Center,
            facing: ArcTextFacing::Outward,
            text: "hashrate".to_owned(),
            style: TextStyle::default(),
        }),
    };

    let still = centre_of(&centred());
    let moved = centre_of(&DrawCommand::Modified {
        animations: vec![HostAnimationDef {
            property: AnimProperty::TranslateY,
            from: 20.0,
            to: 20.0,
            duration_ms: 1_000,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::Forever,
        }],
        transition: None,
        color_space: ColorSpace::default(),
        inner: Box::new(centred()),
    });

    assert_eq!(
        moved,
        (still.0, still.1 + 20.0),
        "the translate outside the Centered reaches the paint"
    );
}

#[test]
fn curved_text_dispatches_inside_outer_rotation() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
    let draw = DrawCommand::CurvedText {
        cx: 5.0,
        cy: 6.0,
        radius: 7.0,
        angle: 0.25,
        anchor: ArcAnchor::Center,
        facing: ArcTextFacing::Outward,
        text: "hashrate".to_owned(),
        style: TextStyle {
            size: 12,
            color: Color::from_rgb(1, 2, 3),
            ..Default::default()
        },
    };

    render_draw_inner_for_test(
        &mut renderer,
        &draw,
        10.0,
        20.0,
        100.0,
        80.0,
        translated(3.0, 4.0),
        0.75,
        2.0,
        1.0,
        0.0,
        None,
        &mut anim_ctx,
    );

    let expected_style = TextStyle {
        size: 24,
        color: Color::from_rgb(1, 2, 3),
        ..Default::default()
    };
    let [
        RenderEvent::Save,
        RenderEvent::Translate(60.0, 60.0),
        RenderEvent::Rotate(0.75),
        RenderEvent::CurvedText {
            cx: -42.0,
            cy: -30.0,
            radius: 14.0,
            angle: 0.25,
            anchor: ArcAnchor::Center,
            facing: ArcTextFacing::Outward,
            text,
            style,
        },
        RenderEvent::Restore,
    ] = &renderer.events[..]
    else {
        panic!("expected rotated curved text draw event");
    };
    assert_eq!(text, "hashrate");
    assert_eq!(style.size, expected_style.size);
    assert_eq!(style.color, expected_style.color);
}

#[test]
fn autofit_text_dispatches_to_renderer() {
    let mut renderer = RecordingRenderer::default();
    let mut animation_states = HashMap::new();
    let mut transition_states = HashMap::new();
    let mut anim_ctx = animation_context(&mut animation_states, &mut transition_states);
    let draw = DrawCommand::AutofitText {
        x: 5.0,
        y: 7.0,
        box_width: 100.0,
        box_height: 40.0,
        mode: AutoFit::Shrink,
        min_size: 16,
        max_size: 0,
        text: "hi".to_string(),
        style: TextStyle {
            size: 32,
            ..TextStyle::default()
        },
    };

    render_draw_inner_for_test(
        &mut renderer,
        &draw,
        0.0,
        0.0,
        200.0,
        100.0,
        DrawOffset::default(),
        0.0,
        1.0,
        1.0,
        0.0,
        None,
        &mut anim_ctx,
    );

    let [
        RenderEvent::AutofitText {
            x,
            y,
            box_width,
            box_height,
            text,
            size,
            mode,
            min_size,
            max_size,
        },
    ] = &renderer.events[..]
    else {
        panic!(
            "expected exactly one AutofitText render event, got: {:?}",
            renderer.events
        );
    };
    assert!((*x - 5.0).abs() < f32::EPSILON);
    assert!((*y - 7.0).abs() < f32::EPSILON);
    assert!((*box_width - 100.0).abs() < f32::EPSILON);
    assert!((*box_height - 40.0).abs() < f32::EPSILON);
    assert_eq!(text, "hi");
    assert_eq!(*size, 32);
    assert_eq!(*mode, AutoFit::Shrink);
    assert_eq!(*min_size, 16);
    assert_eq!(*max_size, 0);
}

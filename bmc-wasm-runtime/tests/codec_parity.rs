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

//! The SDK's own encoder and `bmc-ui`'s agree byte for byte
//! on every node and draw both can build without a host.

#![expect(
    clippy::too_many_lines,
    reason = "one of every node and draw kind, spelled out on each side"
)]

use bmc_ui as ui;
use bmc_ui::{ButtonSkin, NinePatch, SliderSkin};
use bmc_wasm_protocol::colors::{BLACK, WHITE};
use bmc_wasm_protocol::{
    AnimProperty, ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, AutoFit, BitmapId,
    ButtonSize, ButtonStyle, Color, ColorSpace, Easing, Fill, FontWeight, LoopMode, MeshId,
    PropsData, RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments, SkeletonKind, SvgId,
    TagKind, TextStyle,
};
use bmc_wasm_sdk as sdk;

const RED: Color = Color::from_rgb(0xFF, 0, 0);
const BLUE: Color = Color::from_rgb(0, 0, 0xFF);

fn label_style() -> TextStyle {
    TextStyle {
        size: 22,
        weight: FontWeight::BOLD,
        ..TextStyle::default()
    }
}

fn button_skin() -> ButtonSkin {
    ButtonSkin {
        normal: NinePatch::from_id(BitmapId::from_wire(1).expect("BUG: nonzero"), 1, 2, 3, 4),
        pressed: Some(NinePatch::from_id(
            BitmapId::from_wire(2).expect("BUG: nonzero"),
            4,
            3,
            2,
            1,
        )),
        text_color: WHITE,
        pressed_text_color: BLACK,
        opaque: true,
    }
}

fn slider_skin() -> SliderSkin {
    SliderSkin {
        track: NinePatch::from_id(BitmapId::from_wire(3).expect("BUG: nonzero"), 2, 2, 2, 2),
        track_h: 12,
        thumb_id: BitmapId::from_wire(4),
        thumb_w: 16,
        thumb_h: 16,
        thumb_pressed_id: None,
    }
}

fn rel_time_format() -> RelTimeFormat {
    RelTimeFormat {
        length: RelTimeLength::Long,
        segments: RelTimeSegments::Double,
    }
}

fn gap_props() -> PropsData {
    PropsData {
        gap: 8.0,
        ..PropsData::default()
    }
}

fn list_props() -> PropsData {
    PropsData {
        height: 100.0,
        ..PropsData::default()
    }
}

fn ui_draws() -> Vec<ui::Draw> {
    vec![
        ui::Draw::rect(1.0, 2.0, 3.0, 4.0, Fill::linear(90.0, RED, BLUE)),
        ui::Draw::circle(5.0, 6.0, 7.0, RED),
        ui::Draw::arc(
            1.0,
            2.0,
            3.0,
            0.25,
            1.5,
            4.0,
            ArcFill::gradient(RED, BLUE),
            ArcSegments::Explicit(vec![(0.0, 0.25), (0.5, 1.0)]),
            ArcCap::Butt,
        ),
        ui::Draw::centered(ui::Draw::orbit(
            10.0,
            0.5,
            ui::Draw::rotated(1.0, ui::Draw::circle(0.0, 0.0, 1.0, RED)),
        )),
        ui::Draw::rect(0.0, 0.0, 10.0, 10.0, RED).with_drop_shadow(ui::DropShadow {
            dx: 1.0,
            dy: 2.0,
            blur: 4.0,
            color: Color::from_rgba(0, 0, 0, 0x80),
        }),
        ui::Draw::svg_id(0.0, 0.0, 16.0, 16.0, SvgId::from_wire(3), RED)
            .with_anti_alias()
            .fill("rim", BLUE),
        ui::Draw::bitmap_id(0.0, 0.0, 8.0, 8.0, BitmapId::from_wire(0xA55A)),
        ui::Draw::qr(
            1.0,
            2.0,
            40.0,
            "https://example.test",
            ui::QrStyle::default(),
        ),
        ui::Draw::rect(0.0, 0.0, 1.0, 1.0, RED)
            .animate(
                AnimProperty::Rotate,
                0.0,
                1.0,
                1000,
                Easing::EaseOutBack,
                LoopMode::PingPong,
            )
            .animate_color(RED, BLUE, 500, Easing::Linear, LoopMode::Once)
            .transition_with_color_space("hour-hand", 300, Easing::EaseInOut, ColorSpace::Oklch),
        ui::Draw::path(
            vec![(0.0, 0.0), (10.0, 10.0)],
            2.0,
            RED,
            true,
            ui::Interpolation::CatmullRom,
        ),
        ui::Draw::dashed_path(vec![(0.0, 0.0), (10.0, 0.0)], 1.0, RED, 6.0, 4.0),
        ui::Draw::fill_path(
            vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)],
            Fill::radial(RED, BLUE),
            true,
        ),
        ui::Draw::text(10.0, 20.0, "Hello", label_style()),
        ui::Draw::curved_text(
            195.0,
            195.0,
            120.0,
            1.0,
            ArcAnchor::End,
            ArcTextFacing::Inward,
            "MINING",
            label_style(),
        ),
        ui::Draw::autofit_text_ranged(
            4.0,
            8.0,
            200.0,
            80.0,
            "Many names",
            label_style(),
            AutoFit::ShrinkAndGrow,
            14,
            64,
        ),
        ui::Draw::nine_patch_id(
            0,
            0,
            100,
            40,
            NinePatch::from_id(BitmapId::from_wire(2).expect("BUG: nonzero"), 4, 4, 4, 4),
        ),
        ui::Draw::sphere_id(
            0.0,
            0.0,
            400.0,
            400.0,
            BitmapId::from_wire(5),
            50.0,
            14.0,
            1.8,
            Some((20.0, 30.0)),
            true,
        ),
        ui::Draw::mesh_id(
            0.0,
            0.0,
            200.0,
            200.0,
            MeshId::from_wire(9),
            ui::MeshView::default(),
        ),
    ]
}

fn sdk_draws() -> Vec<sdk::Draw> {
    vec![
        sdk::Draw::rect(1.0, 2.0, 3.0, 4.0, Fill::linear(90.0, RED, BLUE)),
        sdk::Draw::circle(5.0, 6.0, 7.0, RED),
        sdk::Draw::arc(
            1.0,
            2.0,
            3.0,
            0.25,
            1.5,
            4.0,
            ArcFill::gradient(RED, BLUE),
            ArcSegments::Explicit(vec![(0.0, 0.25), (0.5, 1.0)]),
            ArcCap::Butt,
        ),
        sdk::Draw::centered(sdk::Draw::orbit(
            10.0,
            0.5,
            sdk::Draw::rotated(1.0, sdk::Draw::circle(0.0, 0.0, 1.0, RED)),
        )),
        sdk::Draw::rect(0.0, 0.0, 10.0, 10.0, RED).with_drop_shadow(sdk::DropShadow {
            dx: 1.0,
            dy: 2.0,
            blur: 4.0,
            color: Color::from_rgba(0, 0, 0, 0x80),
        }),
        sdk::Draw::svg_builtin(0.0, 0.0, 16.0, 16.0, SvgId::from_wire(3), RED)
            .with_anti_alias()
            .fill("rim", BLUE),
        sdk::Draw::bitmap_id(0.0, 0.0, 8.0, 8.0, BitmapId::from_wire(0xA55A)),
        sdk::Draw::qr(
            1.0,
            2.0,
            40.0,
            "https://example.test",
            sdk::QrStyle::default(),
        ),
        sdk::Draw::rect(0.0, 0.0, 1.0, 1.0, RED)
            .animate(
                AnimProperty::Rotate,
                0.0,
                1.0,
                1000,
                Easing::EaseOutBack,
                LoopMode::PingPong,
            )
            .animate_color(RED, BLUE, 500, Easing::Linear, LoopMode::Once)
            .transition_with_color_space("hour-hand", 300, Easing::EaseInOut, ColorSpace::Oklch),
        sdk::Draw::path(
            vec![(0.0, 0.0), (10.0, 10.0)],
            2.0,
            RED,
            true,
            sdk::Interpolation::CatmullRom,
        ),
        sdk::Draw::dashed_path(vec![(0.0, 0.0), (10.0, 0.0)], 1.0, RED, 6.0, 4.0),
        sdk::Draw::fill_path(
            vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)],
            Fill::radial(RED, BLUE),
            true,
        ),
        sdk::Draw::text(10.0, 20.0, "Hello", label_style()),
        sdk::Draw::curved_text(
            195.0,
            195.0,
            120.0,
            1.0,
            ArcAnchor::End,
            ArcTextFacing::Inward,
            "MINING",
            label_style(),
        ),
        sdk::Draw::autofit_text_ranged(
            4.0,
            8.0,
            200.0,
            80.0,
            "Many names",
            label_style(),
            AutoFit::ShrinkAndGrow,
            14,
            64,
        ),
        sdk::Draw::nine_patch_id(
            0,
            0,
            100,
            40,
            NinePatch::from_id(BitmapId::from_wire(2).expect("BUG: nonzero"), 4, 4, 4, 4),
        ),
        sdk::Draw::Sphere {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 400.0,
            bitmap_id: BitmapId::from_wire(5),
            atmosphere: true,
            center_lat: 50.0,
            center_lon: 14.0,
            zoom: 1.8,
            light_lat: 20.0,
            light_lon: 30.0,
        },
        // `MeshView::default()` spelled out as the SDK's flat fields.
        sdk::Draw::Mesh {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
            mesh_id: MeshId::from_wire(9),
            fov: 45.0,
            distance: 3.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            px: 0.0,
            py: 0.0,
            pz: 0.0,
            scale: 1.0,
            light_pitch: f32::NAN,
            light_yaw: f32::NAN,
            ambient: 0.3,
            specular: 0.4,
            hl_u_min: f32::NAN,
            hl_v_min: 0.0,
            hl_u_max: 0.0,
            hl_v_max: 0.0,
            hl_r: 0.0,
            hl_g: 0.0,
            hl_b: 0.0,
        },
    ]
}

fn ui_tree() -> ui::Node {
    let progress = |mode: ui::ProgressMode, key: Option<&str>, skin: Option<SliderSkin>| {
        ui::Node::ProgressBar {
            touch_key: key.map(str::to_owned),
            track_h: 4.0,
            mode,
            active: true,
            fill_color: WHITE,
            track_color: BLACK,
            bg_color: Color::default(),
            skin,
        }
    };
    ui::col(
        PropsData::default(),
        [
            ui::row(gap_props(), [ui::spacer(1.0), ui::spacer(2)]),
            ui::Node::Paragraph {
                props: PropsData::default(),
                base_style: TextStyle::default(),
                spans: vec![
                    ui::Span::plain("plain "),
                    ui::Span {
                        text: "styled".to_owned(),
                        weight: Some(FontWeight::BOLD),
                        color: Some(WHITE),
                        size: Some(24),
                        italic: true,
                        underline: true,
                        strikethrough: true,
                    },
                ],
            },
            ui::make_button(
                "ok",
                "OK",
                ButtonStyle::Ghost,
                ButtonSize::Large,
                SvgId::from_wire(3),
                true,
                Some(button_skin()),
            )
            .stretch(),
            ui::make_button(
                "plain",
                "Plain",
                ButtonStyle::Primary,
                ButtonSize::Normal,
                None,
                false,
                None,
            ),
            ui::canvas(PropsData::default(), ui_draws()),
            ui::touchable("tap", PropsData::default(), Vec::new()),
            ui::Node::Notification {
                kind: ui::NotificationKind::Warning,
                title: "Title".to_owned(),
                subtitle: "Sub".to_owned(),
            },
            ui::Node::RelTime {
                anchor: 1_700_000_000,
                format: rel_time_format(),
                clamp: RelTimeClamp::RemainingOnly,
                style: TextStyle::default(),
            },
            ui::Node::Tag {
                kind: TagKind::Error,
                icon: ui::TagIcon::Custom(SvgId::from_wire(8).expect("BUG: nonzero")),
                content: Box::new(ui::spacer(0)),
            },
            ui::Node::Tag {
                kind: TagKind::Info,
                icon: ui::TagIcon::Default,
                content: Box::new(ui::spacer(0)),
            },
            ui::Node::Skeleton(ui::SkeletonData {
                kind: SkeletonKind::Heading,
                chars: 12.0,
                font_size: 24.0,
                width: 0.0,
                height: 0.0,
                color: WHITE,
            }),
            ui::scroll("list", list_props(), [ui::spacer(1.0)]),
            ui::Node::Modal {
                modal_id: "m".to_owned(),
                is_open: true,
                title: "Modal".to_owned(),
                content_height: 120.0,
                padding: 16,
                backdrop_alpha: 200,
                bg_color: BLACK,
                header_color: WHITE,
                title_color: WHITE,
                max_width: 300,
                body: vec![
                    ui::spacer(1.0),
                    progress(ui::ProgressMode::Meter(0.5), None, None),
                ],
                footer_primary_key: "ok".to_owned(),
                footer_primary_label: "OK".to_owned(),
                footer_secondary_key: "cancel".to_owned(),
                footer_secondary_label: "Cancel".to_owned(),
                footer_danger: true,
            },
            progress(
                ui::ProgressMode::Slider(0.25),
                Some("volume"),
                Some(slider_skin()),
            ),
            progress(ui::ProgressMode::Indeterminate, None, None),
        ],
    )
}

fn sdk_tree() -> sdk::Node {
    let progress = |mode: sdk::ProgressMode, key: &str, skin: Option<SliderSkin>| {
        sdk::progress_bar(key, 4.0, mode, true, WHITE, BLACK, Color::default(), skin)
    };
    sdk::col(
        PropsData::default(),
        [
            sdk::row(gap_props(), [sdk::spacer(1.0), sdk::spacer(2)]),
            sdk::Node::Paragraph {
                props: PropsData::default(),
                base_style: TextStyle::default(),
                spans: vec![
                    sdk::span("plain ", ()),
                    sdk::Span {
                        text: "styled".to_owned(),
                        weight: Some(FontWeight::BOLD),
                        color: Some(WHITE),
                        size: Some(24),
                        italic: true,
                        underline: true,
                        strikethrough: true,
                    },
                ],
            },
            sdk::make_button(
                "ok".to_owned(),
                "OK".to_owned(),
                ButtonStyle::Ghost,
                ButtonSize::Large,
                SvgId::from_wire(3),
                true,
                Some(button_skin()),
            )
            .stretch(),
            sdk::make_button(
                "plain".to_owned(),
                "Plain".to_owned(),
                ButtonStyle::Primary,
                ButtonSize::Normal,
                None,
                false,
                None,
            ),
            sdk::canvas(PropsData::default(), sdk_draws()),
            sdk::touchable("tap", PropsData::default(), Vec::new()),
            sdk::Node::Notification {
                kind: sdk::NotificationKind::Warning,
                title: "Title".to_owned(),
                subtitle: "Sub".to_owned(),
            },
            sdk::Node::RelTime {
                anchor: 1_700_000_000,
                format: rel_time_format(),
                clamp: RelTimeClamp::RemainingOnly,
                style: TextStyle::default(),
            },
            sdk::Node::Tag {
                kind: TagKind::Error,
                icon: sdk::TagIcon::Custom(SvgId::from_wire(8).expect("BUG: nonzero")),
                content: Box::new(sdk::spacer(0)),
            },
            sdk::Node::Tag {
                kind: TagKind::Info,
                icon: sdk::TagIcon::Default,
                content: Box::new(sdk::spacer(0)),
            },
            sdk::Node::Skeleton {
                kind: SkeletonKind::Heading,
                chars: 12.0,
                font_size: 24.0,
                width: 0.0,
                height: 0.0,
                color: WHITE,
            },
            sdk::scroll("list", list_props(), [sdk::spacer(1.0)]),
            sdk::Node::Modal {
                modal_id: "m".to_owned(),
                is_open: true,
                title: "Modal".to_owned(),
                content_height: 120.0,
                padding: 16,
                backdrop_alpha: 200,
                bg_color: BLACK,
                header_color: WHITE,
                title_color: WHITE,
                max_width: 300,
                body: vec![
                    sdk::spacer(1.0),
                    progress(sdk::ProgressMode::Meter(0.5), "", None),
                ],
                footer_primary_key: "ok".to_owned(),
                footer_primary_label: "OK".to_owned(),
                footer_secondary_key: "cancel".to_owned(),
                footer_secondary_label: "Cancel".to_owned(),
                footer_danger: true,
            },
            progress(
                sdk::ProgressMode::Slider(0.25),
                "volume",
                Some(slider_skin()),
            ),
            progress(sdk::ProgressMode::Indeterminate, "", None),
        ],
    )
}

#[test]
fn the_sdk_and_bmc_ui_encode_the_same_tree_to_the_same_bytes() {
    assert_eq!(
        sdk::serialize_node_to_bytes(&sdk_tree()),
        ui::encode(&ui_tree())
    );
}

#[test]
fn what_the_sdk_encodes_bmc_ui_decodes_to_the_same_bytes() {
    let bytes = sdk::serialize_node_to_bytes(&sdk_tree());
    let decoded = ui::decode(&bytes).expect("BUG: the SDK writes a valid tree");
    assert_eq!(decoded.consumed, bytes.len());
    assert_eq!(ui::encode(&decoded.root), bytes);
}

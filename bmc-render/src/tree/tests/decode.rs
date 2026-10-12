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

/// A negative extent paints mirrored geometry that `damage` reports nothing
/// for — see `read_extent`.
#[test]
fn a_negative_extent_is_refused_at_the_boundary() {
    let mut data = vec![DRAW_RECT];
    data.extend_from_slice(&100.0_f32.to_le_bytes());
    data.extend_from_slice(&0.0_f32.to_le_bytes());
    data.extend_from_slice(&(-50.0_f32).to_le_bytes());
    data.extend_from_slice(&10.0_f32.to_le_bytes());
    let mut reader = TreeReader::new(&data);
    let err = reader
        .read_draw()
        .expect_err("a negative width must not reach the renderer")
        .to_string();
    assert!(err.contains("invalid extent"), "wrong rejection: {err}");
}

#[test]
fn a_negative_gap_is_refused_at_the_boundary() {
    let props = PropsData {
        gap: -1.0,
        ..PropsData::default()
    };
    let bytes = props.to_bytes();
    let err = TreeReader::new(&bytes)
        .read_props()
        .expect_err("a negative gap must not reach layout")
        .to_string();
    assert!(
        err.contains("invalid or truncated props"),
        "wrong rejection: {err}"
    );
}

#[test]
fn a_negative_spacer_flex_is_refused_at_the_boundary() {
    let mut data = vec![NODE_SPACER];
    data.extend_from_slice(&(-1.0_f32).to_le_bytes());
    let err = TreeReader::new(&data)
        .read_node()
        .expect_err("a negative flex must not reach layout")
        .to_string();
    assert!(err.contains("invalid extent"), "wrong rejection: {err}");
}

/// `f32::min`/`max` ignore NaN, so one in a rotation range leaves the sweep
/// at `+INF`/`-INF` and the draw reports no damage while still painting —
/// permanently, since a transition keeps it in its recorded `from`.
#[test]
fn a_non_finite_extent_is_refused_at_the_boundary() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut data = vec![DRAW_RECT];
        data.extend_from_slice(&0.0_f32.to_le_bytes());
        data.extend_from_slice(&0.0_f32.to_le_bytes());
        data.extend_from_slice(&bad.to_le_bytes());
        data.extend_from_slice(&10.0_f32.to_le_bytes());
        let mut reader = TreeReader::new(&data);
        let err = reader
            .read_draw()
            .expect_err("a non-finite extent must not reach the tree")
            .to_string();
        assert!(err.contains("extent"), "wrong rejection for {bad}: {err}");
    }
}

#[test]
fn rect_round_trips_a_radial_fill() {
    let mut data = vec![DRAW_RECT];
    data.extend_from_slice(&1.0_f32.to_le_bytes());
    data.extend_from_slice(&2.0_f32.to_le_bytes());
    data.extend_from_slice(&3.0_f32.to_le_bytes());
    data.extend_from_slice(&4.0_f32.to_le_bytes());
    let red = Color::from_rgb(0xFF, 0, 0);
    let blue = Color::from_rgb(0, 0, 0xFF);
    bmc_wasm_protocol::encode_fill(&mut data, &Fill::radial(red, blue));

    let mut de = TreeReader::new(&data);
    let cmd = de
        .read_draw()
        .expect("BUG: test buffer encodes a valid DRAW_RECT");
    let DrawCommand::Rect { fill, .. } = cmd else {
        panic!("expected Rect, got {cmd:?}");
    };
    assert_eq!(fill, Fill::radial(red, blue));
}

#[test]
fn circle_round_trips_a_linear_fill() {
    let mut data = vec![DRAW_CIRCLE];
    data.extend_from_slice(&5.0_f32.to_le_bytes());
    data.extend_from_slice(&6.0_f32.to_le_bytes());
    data.extend_from_slice(&7.0_f32.to_le_bytes());
    let a = Color::from_rgb(1, 2, 3);
    let b = Color::from_rgb(4, 5, 6);
    bmc_wasm_protocol::encode_fill(&mut data, &Fill::linear(0.0, a, b));

    let mut de = TreeReader::new(&data);
    let DrawCommand::Circle { cx, cy, r, fill } = de
        .read_draw()
        .expect("BUG: test buffer encodes a valid DRAW_CIRCLE")
    else {
        panic!("expected Circle");
    };
    assert_eq!((cx, cy, r), (5.0, 6.0, 7.0));
    assert_eq!(fill, Fill::linear(0.0, a, b));
}

#[test]
fn path_fill_round_trips_a_linear_fill() {
    let mut data = vec![DRAW_PATH];
    data.push(0x04);
    data.extend_from_slice(&2_u16.to_le_bytes());
    for (x, y) in [(0.0_f32, 0.0_f32), (10.0, 10.0)] {
        data.extend_from_slice(&x.to_le_bytes());
        data.extend_from_slice(&y.to_le_bytes());
    }
    let a = Color::from_rgb(1, 2, 3);
    let b = Color::from_rgb(4, 5, 6);
    bmc_wasm_protocol::encode_fill(&mut data, &Fill::linear(0.0, a, b));

    let mut de = TreeReader::new(&data);
    let DrawCommand::Path { paint, .. } = de
        .read_draw()
        .expect("BUG: test buffer encodes a valid DRAW_PATH")
    else {
        panic!("expected Path");
    };
    assert_eq!(paint, PathPaint::Fill(Fill::linear(0.0, a, b)));
}

#[test]
fn path_stroke_round_trips_a_dash_pattern() {
    let mut data = vec![DRAW_PATH];
    data.push(0x08); // stroke (not fill) + dashed
    data.extend_from_slice(&2_u16.to_le_bytes());
    for (x, y) in [(0.0_f32, 0.0_f32), (10.0, 0.0)] {
        data.extend_from_slice(&x.to_le_bytes());
        data.extend_from_slice(&y.to_le_bytes());
    }
    let raw = 0x1122_3344_u32;
    data.extend_from_slice(&raw.to_le_bytes()); // color
    data.extend_from_slice(&3.0_f32.to_le_bytes()); // width
    data.extend_from_slice(&6.0_f32.to_le_bytes()); // dash on
    data.extend_from_slice(&4.0_f32.to_le_bytes()); // dash off

    let mut de = TreeReader::new(&data);
    let DrawCommand::Path { paint, .. } = de
        .read_draw()
        .expect("BUG: test buffer encodes a valid dashed DRAW_PATH")
    else {
        panic!("expected Path");
    };
    assert_eq!(
        paint,
        PathPaint::Stroke {
            color: Color::from_raw(raw),
            width: 3.0,
            dash: Some(Dash { on: 6.0, off: 4.0 }),
        }
    );
}

#[test]
fn qr_round_trips_geometry_style_and_text() {
    let mut data = vec![DRAW_QR];
    data.extend_from_slice(&1.0_f32.to_le_bytes()); // x
    data.extend_from_slice(&2.0_f32.to_le_bytes()); // y
    data.extend_from_slice(&40.0_f32.to_le_bytes()); // size
    data.extend_from_slice(&BLACK.to_u32().to_le_bytes()); // dark
    data.extend_from_slice(&WHITE.to_u32().to_le_bytes()); // light
    data.push(3); // quiet_zone
    data.extend_from_slice(&2_u16.to_le_bytes()); // text len
    data.extend_from_slice(b"hi");

    let mut de = TreeReader::new(&data);
    let DrawCommand::Qr {
        x,
        y,
        size,
        dark,
        light,
        quiet_zone,
        text,
    } = de
        .read_draw()
        .expect("BUG: test buffer encodes a valid DRAW_QR")
    else {
        panic!("expected Qr");
    };
    assert_eq!((x, y, size), (1.0, 2.0, 40.0));
    assert_eq!(
        (dark.to_u32(), light.to_u32()),
        (BLACK.to_u32(), WHITE.to_u32())
    );
    assert_eq!(quiet_zone, 3);
    assert_eq!(text, "hi");
}

#[test]
fn autofit_text_decodes_geometry_mode_bounds_style_and_text() {
    let style = TextStyle {
        size: 40,
        ..TextStyle::default()
    };
    let mut data = vec![DRAW_AUTOFIT_TEXT];
    data.extend_from_slice(&10.0_f32.to_le_bytes());
    data.extend_from_slice(&20.0_f32.to_le_bytes());
    data.extend_from_slice(&100.0_f32.to_le_bytes());
    data.extend_from_slice(&50.0_f32.to_le_bytes());
    data.push(AutoFit::ShrinkAndGrow as u8);
    data.extend_from_slice(&14_u16.to_le_bytes());
    data.extend_from_slice(&64_u16.to_le_bytes());
    data.extend_from_slice(&style.to_bytes());
    data.extend_from_slice(&5_u16.to_le_bytes());
    data.extend_from_slice(b"HELLO");

    let mut reader = TreeReader::new(&data);
    let cmd = reader.read_draw().expect("BUG: decode");
    let DrawCommand::AutofitText {
        x,
        y,
        box_width,
        box_height,
        mode,
        min_size,
        max_size,
        text,
        style: s,
    } = cmd
    else {
        panic!("BUG: expected AutofitText");
    };
    assert_eq!((x, y, box_width, box_height), (10.0, 20.0, 100.0, 50.0));
    assert_eq!(mode, AutoFit::ShrinkAndGrow);
    assert_eq!((min_size, max_size), (14, 64));
    assert_eq!(text, "HELLO");
    assert_eq!(s.size, 40);
}

#[test]
fn test_props_size() {
    // In-memory size may differ from wire SIZE due to alignment padding
    assert!(std::mem::size_of::<PropsData>() >= PropsData::SIZE);
}

#[test]
fn read_draw_decodes_shadow() {
    // Wire layout: [DRAW_SHADOW][dx f32][dy f32][blur f32][color u32][inner draw]
    let mut bytes = vec![DRAW_SHADOW];
    bytes.extend_from_slice(&2.0_f32.to_le_bytes());
    bytes.extend_from_slice(&3.0_f32.to_le_bytes());
    bytes.extend_from_slice(&7.5_f32.to_le_bytes());
    bytes.extend_from_slice(&0x1122_3344_u32.to_le_bytes());
    // Inner: a plain rect — [DRAW_RECT][x][y][w][h][fill].
    bytes.push(DRAW_RECT);
    for v in [1.0_f32, 1.0, 4.0, 5.0] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bmc_wasm_protocol::encode_fill(&mut bytes, &Fill::Solid(Color::from_raw(0xFF00_FF00)));

    let draw = TreeReader::new(&bytes)
        .read_draw()
        .expect("BUG: test buffer encodes a valid DRAW_SHADOW");
    let DrawCommand::Shadow {
        dx,
        dy,
        blur,
        color,
        inner,
    } = draw
    else {
        panic!("expected DrawCommand::Shadow, got {draw:?}");
    };
    assert_eq!((dx, dy, blur), (2.0, 3.0, 7.5));
    assert_eq!(color, Color::from_raw(0x1122_3344));
    assert!(
        matches!(*inner, DrawCommand::Rect { .. }),
        "shadow inner should decode as the wrapped rect",
    );
}

#[test]
fn read_draw_decodes_arc() {
    let red = Color::from_rgb(0xFF, 0x00, 0x00);
    let teal = Color::from_rgb(0x00, 0x80, 0x80);
    let expected_fill = ArcFill::gradient(red, teal);
    let expected_segments = ArcSegments::Explicit(vec![(0.0, 0.5), (0.6, 1.5)]);

    let mut data = vec![DRAW_ARC];
    for value in [1.0_f32, 2.0, 30.0, 0.0, 1.5, 6.0] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    bmc_wasm_protocol::encode_arc_fill(&mut data, &expected_fill);
    bmc_wasm_protocol::encode_arc_segments(&mut data, &expected_segments);
    bmc_wasm_protocol::encode_arc_cap(&mut data, ArcCap::Butt);

    let DrawCommand::Arc {
        cx,
        cy,
        radius,
        start_angle,
        end_angle,
        width,
        fill,
        segments,
        cap,
    } = TreeReader::new(&data)
        .read_draw()
        .expect("BUG: test buffer encodes a valid DRAW_ARC")
    else {
        panic!("BUG: expected DrawCommand::Arc");
    };

    assert_eq!((cx, cy, radius), (1.0, 2.0, 30.0));
    assert_eq!((start_angle, end_angle, width), (0.0, 1.5, 6.0));
    assert_eq!(fill, expected_fill);
    assert_eq!(segments, expected_segments);
    assert_eq!(cap, ArcCap::Butt);
}

/// A span's colour and size are both an optional `u32`.
///
/// Only their order on the wire tells them apart, so what pins
/// that order down is decoding a span that carries both.
#[test]
fn a_span_decodes_its_colour_beside_its_size() {
    const COLOR_RAW: u32 = 0x08BD_BAFF;
    const SIZE: u32 = 24;
    // Literal words, not the packer's: weight 700 in bits 0–11,
    // has_weight 12, has_color 13, underline 15; has_size is extra bit 1.
    const FLAGS: u16 = 0x2BC | 0x1000 | 0x2000 | 0x8000;
    const EXTRA: u8 = 0x02;
    let text = "PH/s";

    let mut data = Vec::new();
    data.extend_from_slice(&FLAGS.to_le_bytes());
    data.push(EXTRA);
    data.extend_from_slice(
        &u16::try_from(text.len())
            .expect("BUG: fits u16")
            .to_le_bytes(),
    );
    data.extend_from_slice(text.as_bytes());
    data.extend_from_slice(&COLOR_RAW.to_le_bytes());
    data.extend_from_slice(&SIZE.to_le_bytes());

    let mut reader = TreeReader::new(&data);
    let span = reader
        .read_span()
        .expect("BUG: test buffer encodes a valid span");

    assert_eq!(span.text, text);
    assert_eq!(span.weight, Some(FontWeight::BOLD));
    assert_eq!(
        span.color,
        Some(Color::from_raw(COLOR_RAW)),
        "the colour read back as something else — the two u32s swapped"
    );
    assert_eq!(
        span.size,
        Some(SIZE),
        "the size read back as something else — the two u32s swapped"
    );
    assert!(span.underline);
    assert!(!span.italic);
    assert!(!span.strikethrough);
}

/// A span written before spans carried a size decodes as it always did:
/// no size bit, so nothing is read past its own record.
///
/// That is the compatibility the SDK version promises
/// — a new host reads every old widget; the reverse is not owed.
/// The bytes are literal, as that writer emitted them, with every flag
/// it knew set: a packer that moved a bit would still round-trip itself.
#[test]
fn an_old_widgets_span_decodes_unchanged_on_a_new_host() {
    const NEXT_RECORD: u32 = 0xDEAD_BEEF;
    const COLOR_RAW: u32 = 0x1234_5678;
    // weight 700 in bits 0–11, has_weight 12, has_color 13; italic 14 and
    // underline 15 left clear, so a flag that strays onto them is seen.
    // The extra byte was strikethrough alone, in bit 0.
    const OLD_FLAGS: u16 = 0x2BC | 0x1000 | 0x2000;
    const OLD_EXTRA: u8 = 0x01;
    let text = "500,0";

    let mut data = Vec::new();
    data.extend_from_slice(&OLD_FLAGS.to_le_bytes());
    data.push(OLD_EXTRA);
    data.extend_from_slice(
        &u16::try_from(text.len())
            .expect("BUG: fits u16")
            .to_le_bytes(),
    );
    data.extend_from_slice(text.as_bytes());
    data.extend_from_slice(&COLOR_RAW.to_le_bytes());
    data.extend_from_slice(&NEXT_RECORD.to_le_bytes());

    let mut reader = TreeReader::new(&data);
    let span = reader
        .read_span()
        .expect("BUG: test buffer encodes a valid span");

    assert_eq!(span.text, text);
    assert_eq!(span.weight, Some(FontWeight::BOLD));
    assert_eq!(span.color, Some(Color::from_raw(COLOR_RAW)));
    assert!(span.strikethrough);
    assert!(
        !span.italic && !span.underline,
        "a flag read from a bit it was never on"
    );
    assert_eq!(
        span.size, None,
        "a span that never announced a size must not be handed one"
    );
    assert_eq!(
        reader
            .read_u32()
            .expect("BUG: the next record follows the span"),
        NEXT_RECORD,
        "the reader ran past the span's record into the next one"
    );
}

#[test]
fn curved_text_decodes_geometry_enums_style_and_text() {
    let style = TextStyle {
        size: 18,
        color: Color::from_rgb(9, 8, 7),
        ..TextStyle::default()
    };
    let mut data = vec![DRAW_CURVED_TEXT];
    for value in [1.0_f32, 2.0, 44.0, std::f32::consts::PI] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    data.push(u8::from(ArcAnchor::Center));
    data.push(u8::from(ArcTextFacing::Outward));
    data.extend_from_slice(&style.to_bytes());
    data.extend_from_slice(&5_u16.to_le_bytes());
    data.extend_from_slice(b"HELLO");

    let mut de = TreeReader::new(&data);
    let DrawCommand::CurvedText {
        cx,
        cy,
        radius,
        angle,
        anchor,
        facing,
        text,
        style: decoded_style,
    } = de
        .read_draw()
        .expect("BUG: test buffer encodes a valid DRAW_CURVED_TEXT")
    else {
        panic!("expected CurvedText");
    };

    assert_eq!(
        (
            cx.to_bits(),
            cy.to_bits(),
            radius.to_bits(),
            angle.to_bits()
        ),
        (
            1.0_f32.to_bits(),
            2.0_f32.to_bits(),
            44.0_f32.to_bits(),
            std::f32::consts::PI.to_bits(),
        ),
    );
    assert_eq!(anchor, ArcAnchor::Center);
    assert_eq!(facing, ArcTextFacing::Outward);
    assert_eq!(text, "HELLO");
    assert_eq!(decoded_style.color, style.color);
    assert_eq!(decoded_style.size, style.size);
}

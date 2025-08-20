// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::Palette;
use resvg::{tiny_skia, usvg};
use slint::Color;
use svg::Document;
use svg::node::element::{Path, path::Data};

pub(crate) fn draw_canvas(
    width: u32,
    height: u32,
    draw_large: bool,
    stroke_color: &str,
) -> Document {
    let stroke_width = 2;
    let line_division_coef = if draw_large { 3 } else { 2 };

    let document = Document::new()
        .set("viewBox", (0, 0, width, height))
        .set("width", width)
        .set("height", height);

    let x_axis = Path::new()
        .set(
            "d",
            Data::new()
                .move_to((0, height - stroke_width))
                .line_to((width, height - stroke_width)),
        )
        .set("stroke", stroke_color)
        .set("stroke-width", stroke_width);

    let top_line = Path::new()
        .set(
            "d",
            Data::new()
                .move_to((0, stroke_width))
                .line_to((width, stroke_width)),
        )
        .set("stroke", stroke_color)
        .set("stroke-dasharray", "4,4")
        .set("stroke-width", stroke_width);

    let mid_line = Path::new()
        .set(
            "d",
            Data::new()
                .move_to((0, height / line_division_coef))
                .line_to((width, height / line_division_coef)),
        )
        .set("stroke", stroke_color)
        .set("stroke-dasharray", "4,4")
        .set("stroke-width", stroke_width);

    let extra_line = if draw_large {
        Path::new()
            .set(
                "d",
                Data::new()
                    .move_to((0, 2 * height / line_division_coef))
                    .line_to((width, 2 * height / line_division_coef)),
            )
            .set("stroke", stroke_color)
            .set("stroke-dasharray", "4,4")
            .set("stroke-width", stroke_width)
    } else {
        Path::new()
    };

    document
        .add(x_axis)
        .add(top_line)
        .add(mid_line)
        .add(extra_line)
}

pub(crate) fn create_path(
    data: &Vec<f32>,
    width: u32,
    height: u32,
    stroke_color: String,
) -> Option<Path> {
    if data.is_empty() {
        return None;
    }
    // Prevent graph clipping on the screen
    let vertical_margin = 10.0;
    #[expect(clippy::cast_precision_loss)]
    let height = height as f32 - vertical_margin;

    #[expect(clippy::cast_precision_loss)]
    let point_width = width as f32 / (data.len() - 1) as f32;
    let max_value = data
        .iter()
        .max_by(|a, b| a.total_cmp(b))
        .copied()
        .unwrap_or_default();
    let min_value = data
        .iter()
        .min_by(|a, b| a.total_cmp(b))
        .copied()
        .unwrap_or_default();
    let point_ratio = height / (max_value - min_value);

    let point_shift = |index: f32| -> f32 { index * point_width };
    let point_value =
        |value: f32| -> f32 { point_ratio * (max_value - value) + vertical_margin / 2.0 };

    let stroke_width = 3;
    let mut path_data = Data::new();

    for (index, &value) in data.iter().enumerate() {
        if index == 0 {
            path_data = path_data.move_to((0, point_value(value)));
        } else {
            path_data = path_data.line_to((point_shift(index as f32), point_value(value)));
        }
    }

    Some(
        Path::new()
            .set("fill", "none")
            .set("stroke", stroke_color)
            .set("stroke-width", stroke_width)
            .set("d", path_data),
    )
}

pub(crate) struct ColorPalette {
    pub green_50: String,
    pub green_60: String,
    pub green_100: String,
    pub red_50: String,
    pub red_60: String,
    pub red_100: String,
    pub blue_30: String,
    pub violet_60: String,
    pub gray_80: String,
    pub black: String,
}

impl ColorPalette {
    pub(crate) fn new(palette: &Palette<'_>) -> Self {
        Self {
            green_50: Self::color_to_hex(palette.get_green_50().color()),
            green_60: Self::color_to_hex(palette.get_green_60().color()),
            green_100: Self::color_to_hex(palette.get_green_100().color()),
            red_50: Self::color_to_hex(palette.get_red_50().color()),
            red_60: Self::color_to_hex(palette.get_red_60().color()),
            red_100: Self::color_to_hex(palette.get_red_100().color()),
            blue_30: Self::color_to_hex(palette.get_blue_30().color()),
            violet_60: Self::color_to_hex(palette.get_violet_60().color()),
            gray_80: Self::color_to_hex(palette.get_gray_80().color()),
            black: Self::color_to_hex(palette.get_black().color()),
        }
    }

    fn color_to_hex(color: Color) -> String {
        format!(
            "#{:02X}{:02X}{:02X}",
            color.red(),
            color.green(),
            color.blue()
        )
    }
}

pub(crate) fn svg_to_rgb8(svg_data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let opt = usvg::Options::default();
    let rtree = usvg::Tree::from_data(svg_data, &opt).ok()?;

    let mut buffer = vec![0; (width * height * 4) as usize];
    let mut pixmap = tiny_skia::PixmapMut::from_bytes(&mut buffer, width, height)?;

    resvg::render(&rtree, tiny_skia::Transform::identity(), &mut pixmap);

    let raw_rgba = pixmap.data_mut();
    // Convert RGBA to RGB by removing the alpha channel
    let mut rgb_data = Vec::with_capacity((width * height * 3) as usize);
    for chunk in raw_rgba.chunks(4) {
        rgb_data.push(chunk[0]); // R
        rgb_data.push(chunk[1]); // G
        rgb_data.push(chunk[2]); // B
    }

    Some(rgb_data)
}

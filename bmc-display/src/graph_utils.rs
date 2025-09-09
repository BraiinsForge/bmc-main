// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::Palette;
use resvg::{tiny_skia, usvg};
use slint::{Color, Image, Rgba8Pixel, SharedPixelBuffer};
use svg::Document;
use svg::node::element::{Definitions, LinearGradient, Path, Stop, path::Data};

pub(crate) fn draw_canvas(
    width: u32,
    height: u32,
    draw_extra_line: bool,
    stroke_color: &str,
) -> Document {
    let stroke_width = 2;
    let line_division_coef = if draw_extra_line { 3 } else { 2 };

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

    let extra_line = if draw_extra_line {
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

pub(crate) fn create_graph(
    data: &[f32],
    width: u32,
    height: u32,
    stroke_color: &str,
    abs_values: bool,
    units_integer_round: bool,
    shift_max: Option<f32>,
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
    let coef = if abs_values {
        max_value
    } else {
        max_value - min_value
    };

    let shift_max = shift_max.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&shift_max) {
        return None;
    }

    let point_ratio = if coef == 0.0 { height } else { height / coef };
    // Shift Max
    let point_ratio = point_ratio * shift_max;
    let move_max = height * (1.0 - shift_max);

    // New axis max
    let axis_max = y_axis_max(max_value, units_integer_round);
    let new_max_coef = max_value / axis_max;
    let point_ratio = point_ratio * new_max_coef;
    let new_max_move = height * (1.0 - new_max_coef) * shift_max;

    let point_shift = |index: f32| -> f32 { index * point_width };
    let point_value = |value: f32| -> f32 {
        point_ratio * (max_value - value) + move_max + new_max_move + vertical_margin / 2.0
    };

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

pub(crate) fn create_graph_fill(
    data: &[f32],
    width: u32,
    height: u32,
    palette: &Palette<'_>,
    use_gradient: bool,
) -> Image {
    if data.is_empty() {
        return Image::default();
    }

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

    let coef = max_value - min_value;
    let height_f32 = height as f32;
    let point_ratio = if coef == 0.0 {
        height_f32
    } else {
        height_f32 / coef
    };
    let point_shift = |index: f32| -> f32 { index * point_width };
    let point_value = |value: f32| -> f32 { point_ratio * (max_value - value) };

    let border_command = Data::new()
        .move_to((0, 0))
        .move_to((width, 0))
        .move_to((width, height))
        .move_to((0, height));

    let mut path_data = border_command;
    for (index, &value) in data.iter().enumerate() {
        if index == 0 {
            path_data = path_data.move_to((0, point_value(value)));
        } else {
            path_data = path_data.line_to((point_shift(index as f32), point_value(value)));
        }
    }

    path_data = path_data.line_to((width, height)).line_to((0, height));

    let color_palette = ColorPalette::new(&palette);
    let (graph_color, graph_gradient_color, graph_fill_color) = if data.first() <= data.last() {
        (
            color_palette.green_50,
            color_palette.green_60,
            color_palette.green_100,
        )
    } else {
        (
            color_palette.red_50,
            color_palette.red_60,
            color_palette.red_100,
        )
    };
    let background_color = color_palette.black.clone();

    let stop1 = Stop::new()
        .set("offset", "0%")
        .set("stop-color", graph_gradient_color);

    let stop2 = Stop::new()
        .set("offset", "100%")
        .set("stop-color", background_color.clone());

    let gradient = LinearGradient::new()
        .set("id", "Gradient")
        .set("x1", "0%")
        .set("y1", "0%")
        .set("x2", "0%")
        .set("y2", "100%")
        .add(stop1)
        .add(stop2);

    let defs = Definitions::new().add(gradient);

    let stroke_width = 2;

    let partial_path = Path::new()
        .set("stroke", graph_color)
        .set("stroke-width", stroke_width)
        .set("d", path_data);
    let path = if use_gradient {
        partial_path.set("fill", "url(#Gradient)")
    } else {
        partial_path.set("fill", graph_fill_color)
    };

    // Rectangle placed under the graph to match the background color
    let rect_command = Data::new()
        .move_to((0, 0))
        .line_to((width, 0))
        .line_to((width, height))
        .line_to((0, height))
        .close();
    let background_rect = Path::new()
        .set("fill", background_color.clone())
        .set("stroke", background_color.clone())
        .set("stroke-width", stroke_width)
        .set("d", rect_command.clone());
    let overlay_rect = Path::new()
        .set("fill-opacity", 0.0)
        .set("stroke", color_palette.black)
        .set("stroke-width", stroke_width)
        .set("d", rect_command);

    let partial_document = Document::new()
        .set("width", width)
        .set("height", height)
        .set("viewBox", (0, 0, width, height));
    let document = if use_gradient {
        partial_document.add(background_rect).add(defs).add(path)
    } else {
        partial_document.add(path).add(overlay_rect)
    };

    svg_into_image(document, width, height)
}

// Calculates nearest number divisible by 3
pub(crate) fn y_axis_max(value: f32, integer_round: bool) -> f32 {
    if value == 0.0 {
        return 3.0;
    }
    let exponent = if integer_round {
        value.log10().floor()
    } else {
        value.log10().floor() - 1.0
    };
    let divisor = 3.0 * f32::powf(10.0, exponent);
    let multiplicator = (value / divisor).ceil();
    multiplicator * divisor
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

pub(crate) fn blend_svg_with_image(
    svg_document: Document,
    bg_buffer: SharedPixelBuffer<Rgba8Pixel>,
    width: u32,
    height: u32,
) -> Option<Image> {
    let mut svg_image: Vec<u8> = vec![];
    svg::write(&mut svg_image, &svg_document).ok()?;

    let rgba_data = svg_to_rgba8(&svg_image, width, height)?;
    let fg_buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba_data, width, height);

    let fg_pixels = fg_buffer.as_slice();
    let bg_pixels = bg_buffer.as_slice();

    let blended_pixels: Vec<Rgba8Pixel> = fg_pixels
        .iter()
        .zip(bg_pixels.iter())
        .map(|(&fg, &bg)| if fg.a == 255 { fg } else { bg })
        .collect();
    let mut result = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    let result_pixels = result.make_mut_slice();
    result_pixels.copy_from_slice(&blended_pixels);

    Some(Image::from_rgba8(result))
}

fn svg_to_rgba8(svg_data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let opt = usvg::Options::default();
    let rtree = usvg::Tree::from_data(svg_data, &opt).ok()?;

    let mut buffer = vec![0; (width * height * 4) as usize];
    let mut pixmap = tiny_skia::PixmapMut::from_bytes(&mut buffer, width, height)?;

    resvg::render(&rtree, tiny_skia::Transform::identity(), &mut pixmap);

    let raw_rgba = pixmap.data_mut();
    let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);
    for chunk in raw_rgba.chunks(4) {
        rgba_data.push(chunk[0]); // R
        rgba_data.push(chunk[1]); // G
        rgba_data.push(chunk[2]); // B
        rgba_data.push(chunk[3]); // A
    }

    Some(rgba_data)
}

pub(crate) fn svg_into_image(svg_document: Document, width: u32, height: u32) -> Image {
    let mut svg_image: Vec<u8> = vec![];
    if svg::write(&mut svg_image, &svg_document).is_err() {
        return Image::default();
    }

    if let Some(rgba_data) = svg_to_rgba8(&svg_image, width, height) {
        Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &rgba_data, width, height,
        ))
    } else {
        Image::default()
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{MainWindow, Palette};
use bmc_shared_utils::number_format::NumberFormat;
use crate::graph_utils::{self, ColorPalette};
use serde::Deserialize;
use slint::{Global, Image};
use svg::Document;
use svg::node::element::{Definitions, LinearGradient, Path, Stop};

#[derive(Clone, Debug, Deserialize, Default)]
struct PricePoint {
    // Timestamp
    #[expect(dead_code)]
    x: String,
    // BTC Price
    y: f32,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct BtcHistoryData {
    price: Option<Vec<PricePoint>>,
}

impl BtcHistoryData {
    pub(crate) fn graph_image(&self, main_window: &MainWindow, width: u32, height: u32) -> Image {
        let palette = Palette::get(main_window);

        self.price_graph_as_image(width, height, &palette, false)
    }

    // NOTE: Code for later use #BOS-3408
    #[expect(dead_code)]
    pub(crate) fn price_change_frame(&self, number_format: NumberFormat) -> SharedString {
        if let Some((first, last)) = self.first_last_price() {
            let change = 100.0 * f64::from(last / first - 1.0);
            SharedString::from(format!(
                "{}{}%",
                if change.is_sign_positive() { "+" } else { "" },
                number_format.format_number(change)
            ))
        } else {
            SharedString::default()
        }
    }

    // NOTE: Code for later use #BOS-3408
    #[expect(dead_code)]
    pub(crate) fn increasing_trend_frame(&self) -> bool {
        if let Some((first, last)) = self.first_last_price() {
            last > first
        } else {
            false
        }
    }

    // NOTE: Code for later use #BOS-3408
    fn first_last_price(&self) -> Option<(f32, f32)> {
        let prices = match self.price.as_ref() {
            Some(prices) => prices
                .iter()
                .map(|price_point| price_point.y)
                .collect::<Vec<f32>>(),
            None => return None,
        };
        if prices.is_empty() {
            return None;
        }
        match (prices.first(), prices.last()) {
            (Some(first), Some(last)) => Some((*first, *last)),
            _ => None,
        }
    }

    fn price_graph_as_image(
        &self,
        width: u32,
        height: u32,
        palette: &Palette<'_>,
        use_gradient: bool,
    ) -> Image {
        let Some(price) = self.price.as_ref() else {
            return Image::default();
        };

        let prices: Vec<f32> = price.iter().map(|price_point| price_point.y).collect();
        if prices.is_empty() {
            return Image::default();
        }

        #[expect(clippy::cast_precision_loss)]
        let p_width = width as f32 / (prices.len() - 1) as f32;
        let max_price = prices
            .iter()
            .max_by(|a, b| a.total_cmp(b))
            .copied()
            .unwrap_or(100_000.0); // Check for empty vector done above
        let min_price = prices
            .iter()
            .min_by(|a, b| a.total_cmp(b))
            .copied()
            .unwrap_or(0.0); // Check for empty vector done above
        #[expect(clippy::cast_precision_loss)]
        let p_ratio = height as f32 / (max_price - min_price);

        let p_shift = |index: f32| -> f32 { index * p_width };
        let p_value = |price: f32| -> f32 { p_ratio * (max_price - price) };

        let border_command = format!("M 0 0 M {width} 0 M {width} {height} M 0 {height}");
        let mut commands = vec![border_command];

        for (index, &price) in prices.iter().enumerate() {
            commands.push(if index == 0 {
                format!("M 0 {y}", y = p_value(price))
            } else {
                #[expect(clippy::cast_precision_loss)]
                let x = p_shift(index as f32);
                format!("L {x} {y}", y = p_value(price))
            });
        }
        commands.push(format!("L {width} {height} L 0 {height}"));
        let command = commands.join(" ");

        let color_palette = ColorPalette::new(&palette);
        let (graph_color, graph_gradient_color, graph_fill_color) =
            if prices.first() <= prices.last() {
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
            .set("d", command);
        let path = if use_gradient {
            partial_path.set("fill", "url(#Gradient)")
        } else {
            partial_path.set("fill", graph_fill_color)
        };

        // Rectangle placed under the graph to match the background color
        let rect_command = format!("M0,0 L{width},0 L{width},{height} L0,{height} Z");
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

        graph_utils::svg_into_image(document, width, height)
    }
}

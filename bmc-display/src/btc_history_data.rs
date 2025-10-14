// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{MainWindow, Palette};
use crate::graph_utils::{self, ColorPalette};
use bmc_shared_utils::number_format::NumberFormat;
use serde::Deserialize;
use slint::{Global, Image, SharedString};

#[derive(Clone, Debug, Deserialize, Default)]
struct PricePoint {
    // Timestamp
    #[expect(dead_code)]
    x: String,
    // BTC Price
    y: f64,
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

    pub(crate) fn graph_small_image(
        &self,
        main_window: &MainWindow,
        width: u32,
        height: u32,
        draw_extra_line: bool,
    ) -> Image {
        let Some(price) = self.price.as_ref() else {
            // return empty canvas
            return Image::default();
        };
        let data: Vec<f64> = price.iter().map(|price_point| price_point.y).collect();

        let palette = Palette::get(main_window);
        let palette = ColorPalette::new(&palette);

        let canvas =
            graph_utils::draw_canvas(width, height, draw_extra_line, true, &palette.gray_80);
        let color = if self.increasing_trend_frame() {
            palette.green_40
        } else {
            palette.red_50
        };
        let path = graph_utils::create_graph(&data, width, height, &color, false, false, None)
            .unwrap_or_default();
        let document = canvas.add(path);

        graph_utils::svg_into_image(&document, width, height)
    }

    // NOTE: Code for later use #BOS-3408
    #[expect(dead_code)]
    pub(crate) fn price_change_frame(&self, number_format: NumberFormat) -> SharedString {
        if let Some((first, last)) = self.first_last_price() {
            let change = 100.0 * (last / first - 1.0);
            SharedString::from(format!(
                "{}{}%",
                if change.is_sign_positive() { "+" } else { "" },
                number_format.format_number(change, 1)
            ))
        } else {
            SharedString::default()
        }
    }

    fn increasing_trend_frame(&self) -> bool {
        if let Some((first, last)) = self.first_last_price() {
            last > first
        } else {
            false
        }
    }

    fn first_last_price(&self) -> Option<(f64, f64)> {
        let prices = match self.price.as_ref() {
            Some(prices) => prices
                .iter()
                .map(|price_point| price_point.y)
                .collect::<Vec<f64>>(),
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
        let prices: Vec<f64> = price.iter().map(|price_point| price_point.y).collect();

        graph_utils::create_graph_fill(&prices, width, height, palette, use_gradient)
    }
}

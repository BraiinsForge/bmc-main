// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{MainWindow, Palette};
use crate::graph_utils::{self, ColorPalette};
use serde::Deserialize;
use slint::{Global, Image, SharedString};

#[derive(Clone, Debug, Deserialize, Default)]
struct DifficultyPoint {
    // Timestamp
    #[expect(dead_code)]
    x: String,
    // Difficulty
    y: f64,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct HashratePoint {
    // Timestamp
    #[expect(dead_code)]
    x: String,
    // Hashrate
    y: f64,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct HashrateField {
    global: Option<Vec<HashratePoint>>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct DiffHashrateData {
    difficulty: Option<Vec<DifficultyPoint>>,
    hashrate: Option<HashrateField>,
}

impl DiffHashrateData {
    pub(crate) fn graph_hashrate_image(
        &self,
        main_window: &MainWindow,
        width: u32,
        height: u32,
        draw_extra_line: bool,
    ) -> Image {
        let Some(hashrate) = self.hashrate.as_ref() else {
            // return empty canvas
            return Image::default();
        };
        let Some(hashrate) = hashrate.global.as_ref() else {
            // return empty canvas
            return Image::default();
        };
        let data: Vec<f64> = hashrate
            .iter()
            .map(|hashrate_point| hashrate_point.y)
            .collect();

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

    fn increasing_trend_frame(&self) -> bool {
        if let Some((first, last)) = self.first_last_value() {
            last > first
        } else {
            false
        }
    }

    fn first_last_value(&self) -> Option<(f64, f64)> {
        let hashrate = self
            .hashrate
            .as_ref()
            .and_then(|hf| hf.global.as_ref())
            .map(|points| points.iter().map(|point| point.y).collect::<Vec<f64>>())?;

        match (hashrate.first(), hashrate.last()) {
            (Some(first), Some(last)) => Some((*first, *last)),
            _ => None,
        }
    }
}

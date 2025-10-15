// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{MainWindow, Palette};
use crate::graph_utils::{self, ColorPalette};
use bmc_shared_utils::number_format::NumberFormat;
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
        let color = if self.hashrate_increasing_trend() {
            palette.green_40
        } else {
            palette.red_50
        };
        let path = graph_utils::create_graph(&data, width, height, &color, false, false, None)
            .unwrap_or_default();
        let document = canvas.add(path);

        graph_utils::svg_into_image(&document, width, height)
    }

    pub(crate) fn graph_dificulty_image(
        &self,
        main_window: &MainWindow,
        width: u32,
        height: u32,
        draw_extra_line: bool,
    ) -> Image {
        let Some(difficulty) = self.difficulty.as_ref() else {
            return Image::default();
        };
        let data: Vec<f64> = difficulty
            .iter()
            .map(|difficulty_point| difficulty_point.y)
            .collect();

        let palette = Palette::get(main_window);
        let palette = ColorPalette::new(&palette);

        let canvas =
            graph_utils::draw_canvas(width, height, draw_extra_line, true, &palette.gray_80);
        let color = if self.difficulty_increasing_trend() {
            palette.green_40
        } else {
            palette.red_50
        };
        let path = graph_utils::create_graph(&data, width, height, &color, false, false, None)
            .unwrap_or_default();
        let document = canvas.add(path);

        graph_utils::svg_into_image(&document, width, height)
    }

    pub(crate) fn hashrate_increasing_trend(&self) -> bool {
        if let Some((first, last)) = self.hashrate_first_last_value() {
            last > first
        } else {
            false
        }
    }

    pub(crate) fn difficulty_increasing_trend(&self) -> bool {
        if let Some((first, last)) = self.difficulty_first_last_value() {
            last > first
        } else {
            false
        }
    }

    pub(crate) fn hashrate_change_trend(&self, number_format: NumberFormat) -> SharedString {
        if let Some((first, last)) = self.hashrate_first_last_value() {
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

    fn hashrate_first_last_value(&self) -> Option<(f64, f64)> {
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

    fn difficulty_first_last_value(&self) -> Option<(f64, f64)> {
        let difficulties = match self.difficulty.as_ref() {
            Some(difficulties) => difficulties
                .iter()
                .map(|difficulty_point| difficulty_point.y)
                .collect::<Vec<f64>>(),
            None => return None,
        };
        if difficulties.is_empty() {
            return None;
        }
        match (difficulties.first(), difficulties.last()) {
            (Some(first), Some(last)) => Some((*first, *last)),
            _ => None,
        }
    }
}

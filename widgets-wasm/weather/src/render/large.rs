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

use crate::{
    display,
    render::{
        common::{self, BORDER, TEXT_PRIMARY, TEXT_SECONDARY},
        icons,
    },
    weather_code,
};

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

#[derive(Clone, Copy)]
enum CurrentLayout {
    Stacked,
    Compact,
}

#[derive(Clone, Copy)]
struct LargeMetrics {
    current_layout: CurrentLayout,
    location_font_size: u32,
    temperature_font_size: u32,
    current_icon_size: f32,
    padding: f32,
    condition_font_size: u32,
    current_gap: f32,
    compact_icon_text_gap: f32,
    root_gap: f32,
    today_gap: f32,
    stat_item_gap: f32,
    stat_label_gap: f32,
    stat_row_gap: f32,
    stats_panel_gap: f32,
    stat_glyph_size: f32,
    stat_label_font_size: u32,
    stat_value_font_size: u32,
    stat_meridiem_font_size: u32,
    forecast_gap: f32,
    forecast_label_font_size: u32,
    forecast_icon_size: f32,
    forecast_temperature_font_size: u32,
    forecast_temperature_cell_width: f32,
    forecast_bar_width: f32,
    forecast_bar_height: f32,
    forecast_row_gap: f32,
}

impl LargeMetrics {
    fn for_size(size: WidgetSize) -> Self {
        let fit = size.fit();
        let current_layout = if matches!(size.variant, SizeVariant::Large) && size.height < 360 {
            CurrentLayout::Compact
        } else {
            CurrentLayout::Stacked
        };
        let (root_gap, current_gap) = match current_layout {
            CurrentLayout::Stacked => (16.0 * fit, 8.0 * fit),
            CurrentLayout::Compact => (16.0 * fit, 6.0 * fit),
        };
        Self {
            current_layout,
            location_font_size: 16,
            temperature_font_size: scale_font(64, fit),
            current_icon_size: 68.0 * fit,
            padding: 16.0 * fit,
            condition_font_size: scale_font(24, fit),
            current_gap,
            compact_icon_text_gap: 54.0 * fit,
            root_gap,
            today_gap: 24.0 * fit,
            stat_item_gap: 6.0 * fit,
            stat_label_gap: 8.0 * fit,
            stat_row_gap: 24.0 * fit,
            stats_panel_gap: 12.0 * fit,
            stat_glyph_size: 24.0 * fit,
            stat_label_font_size: scale_font(24, fit),
            stat_value_font_size: scale_font(32, fit),
            stat_meridiem_font_size: scale_font(20, fit),
            forecast_gap: 12.0 * fit,
            forecast_label_font_size: scale_font(24, fit),
            forecast_icon_size: 40.0 * fit,
            forecast_temperature_font_size: scale_font(32, fit),
            forecast_temperature_cell_width: 80.0 * fit,
            forecast_bar_width: 140.0 * fit,
            forecast_bar_height: 16.0 * fit,
            forecast_row_gap: 12.0 * fit,
        }
    }

    fn location_max_width(self, size: WidgetSize) -> u32 {
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "widget dimensions and padding are small positive pixel values"
        )]
        {
            ((size.width as f32) - 2.0 * self.padding).max(1.0).round() as u32
        }
    }

    fn forecast_row_style(self) -> common::ForecastRowStyle {
        common::ForecastRowStyle {
            row_gap: self.forecast_row_gap,
            label_size: self.forecast_label_font_size,
            icon_size: self.forecast_icon_size,
            temperature_size: self.forecast_temperature_font_size,
            temperature_cell_width: self.forecast_temperature_cell_width,
            bar_width: self.forecast_bar_width,
            bar_height: self.forecast_bar_height,
        }
    }
}

fn stat_item(svg: &'static Svg, label: &str, value: Node, metrics: LargeMetrics) -> Node {
    col(
        props!(cross_align: CrossAlign::Center, gap: metrics.stat_item_gap),
        [
            row(
                props!(cross_align: CrossAlign::Center, gap: metrics.stat_label_gap),
                [
                    common::glyph(svg, metrics.stat_glyph_size, TEXT_SECONDARY),
                    common::txt(
                        label.to_string(),
                        metrics.stat_label_font_size,
                        FontWeight::REGULAR,
                        TEXT_SECONDARY,
                    ),
                ],
            ),
            value,
        ],
    )
}

fn location_row(weather: &crate::model::Weather, size: WidgetSize, metrics: LargeMetrics) -> Node {
    text(
        weather.location.display_name.clone(),
        style!(
            size: metrics.location_font_size,
            weight: FontWeight::REGULAR,
            color: TEXT_SECONDARY,
            line_height: 1.0,
            max_width: metrics.location_max_width(size),
            text_overflow: TextOverflow::Clip,
        ),
    )
}

fn current_temperature(current: Option<&crate::model::Current>, metrics: LargeMetrics) -> Node {
    common::txt(
        display::temperature_or_placeholder(current.map(|c| c.temperature), display::temperature),
        metrics.temperature_font_size,
        FontWeight::BOLD,
        TEXT_PRIMARY,
    )
}

fn current_condition(current: Option<&crate::model::Current>, metrics: LargeMetrics) -> Node {
    common::txt(
        current.map_or_else(
            || display::NOT_AVAILABLE.to_string(),
            |c| weather_code::description(c.weather_code).to_string(),
        ),
        metrics.condition_font_size,
        FontWeight::REGULAR,
        TEXT_SECONDARY,
    )
}

fn current_left(weather: &crate::model::Weather, metrics: LargeMetrics) -> Node {
    let current = weather.current.as_ref();
    let mut stack: Vec<Node> = Vec::new();
    if let Some(c) = current {
        stack.push(common::weather_icon(
            weather_code::icon_id(c.weather_code, c.is_day),
            metrics.current_icon_size,
        ));
    }
    match metrics.current_layout {
        CurrentLayout::Stacked => {
            stack.push(current_temperature(current, metrics));
            stack.push(current_condition(current, metrics));
            col(
                props!(cross_align: CrossAlign::Start, gap: metrics.current_gap),
                stack,
            )
        }
        CurrentLayout::Compact => {
            stack.push(col(
                props!(cross_align: CrossAlign::Start, gap: metrics.current_gap),
                [
                    current_temperature(current, metrics),
                    current_condition(current, metrics),
                ],
            ));
            row(
                props!(cross_align: CrossAlign::Center, gap: metrics.compact_icon_text_gap),
                stack,
            )
        }
    }
}

fn stats_panel(weather: &crate::model::Weather, tz: Option<&Tz>, metrics: LargeMetrics) -> Node {
    let Some(daily) = &weather.daily else {
        return common::txt(
            display::NOT_AVAILABLE.to_string(),
            metrics.condition_font_size,
            FontWeight::REGULAR,
            TEXT_SECONDARY,
        );
    };
    let today = daily.days.get(daily.today_index);
    let low = today.map_or_else(
        || display::NOT_AVAILABLE.to_string(),
        |d| display::temperature(d.min),
    );
    let high = today.map_or_else(
        || display::NOT_AVAILABLE.to_string(),
        |d| display::temperature(d.max),
    );
    col(
        props!(gap: metrics.stats_panel_gap),
        [
            row(
                props!(gap: metrics.stat_row_gap),
                [
                    stat_item(
                        &icons::TEMP_LOW,
                        "Low T.",
                        common::txt(
                            low,
                            metrics.stat_value_font_size,
                            FontWeight::SEMIBOLD,
                            TEXT_PRIMARY,
                        ),
                        metrics,
                    ),
                    stat_item(
                        &icons::TEMP_HIGH,
                        "High T.",
                        common::txt(
                            high,
                            metrics.stat_value_font_size,
                            FontWeight::SEMIBOLD,
                            TEXT_PRIMARY,
                        ),
                        metrics,
                    ),
                ],
            ),
            row(
                props!(gap: metrics.stat_row_gap),
                [
                    stat_item(
                        &icons::SUNRISE,
                        "Sunrise",
                        common::time_with_meridiem(
                            &daily.today_sunrise,
                            tz,
                            metrics.stat_value_font_size,
                            metrics.stat_meridiem_font_size,
                            FontWeight::SEMIBOLD,
                        ),
                        metrics,
                    ),
                    stat_item(
                        &icons::SUNSET,
                        "Sunset",
                        common::time_with_meridiem(
                            &daily.today_sunset,
                            tz,
                            metrics.stat_value_font_size,
                            metrics.stat_meridiem_font_size,
                            FontWeight::SEMIBOLD,
                        ),
                        metrics,
                    ),
                ],
            ),
        ],
    )
}

#[must_use]
pub fn large(
    weather: &crate::model::Weather,
    params: &crate::manifest_params::Params,
    size: WidgetSize,
) -> Node {
    let tz = display::select_tz(params.time_zone, &weather.location.timezone);
    let metrics = LargeMetrics::for_size(size);

    // Stretch the row so the stat grid can bottom-align (deckfeeder
    // `align-self: end`) against the current block via a leading spacer.
    let today = row(
        props!(cross_align: CrossAlign::Stretch, gap: metrics.today_gap),
        [
            current_left(weather, metrics),
            spacer(1.0),
            col(
                props!(),
                [spacer(1.0), stats_panel(weather, tz.as_ref(), metrics)],
            ),
        ],
    );

    let forecast = if let Some(daily) = &weather.daily {
        let window = daily.forecast_window(4);
        let range = crate::model::ForecastRange::of(window);
        let rows: Vec<Node> = window
            .iter()
            .enumerate()
            .map(|(i, day)| {
                let is_today = i == 0;
                let marker = if is_today {
                    weather.current.as_ref().map(|c| c.temperature)
                } else {
                    None
                };
                common::forecast_row(day, is_today, &range, marker, metrics.forecast_row_style())
            })
            .collect();
        col(props!(gap: metrics.forecast_gap), rows)
    } else {
        col(
            props!(),
            [common::txt(
                display::NOT_AVAILABLE.to_string(),
                24,
                FontWeight::REGULAR,
                TEXT_SECONDARY,
            )],
        )
    };

    col(
        props!(background: BLACK, flex: 1.0, padding: metrics.padding, gap: metrics.root_gap),
        [
            location_row(weather, size, metrics),
            today,
            col(props!(height: 1.0, background: BORDER), []),
            forecast,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{LargeMetrics, large};
    use crate::{
        manifest_params::{Params, TimeZone},
        model::{Current, Daily, DayForecast, Location, Weather},
    };
    use bmc_wasm_sdk::assets::init_test_registrars;
    use bmc_wasm_sdk::{Node, TextOverflow, WidgetSize};
    use units::{availability::Availability, units::DegreeCelsius};

    #[test]
    fn bmm101_large_metrics_scale_by_fit() {
        let metrics = LargeMetrics::for_size(WidgetSize::from_dimensions(480, 320));

        assert_eq!(metrics.location_font_size, 16);
        assert_eq!(metrics.temperature_font_size, 43);
        assert_eq!(metrics.current_icon_size, 45.333_336);
        assert_eq!(metrics.padding, 10.666_667);
        assert_eq!(metrics.forecast_bar_width, 93.333_336);
    }

    #[test]
    fn canonical_large_metrics_stay_authored() {
        let metrics = LargeMetrics::for_size(WidgetSize::from_dimensions(638, 480));

        assert_eq!(metrics.location_font_size, 16);
        assert_eq!(metrics.temperature_font_size, 64);
        assert_eq!(metrics.current_icon_size, 68.0);
        assert_eq!(metrics.padding, 16.0);
        assert_eq!(metrics.forecast_bar_width, 140.0);
    }

    #[test]
    fn bmm101_forecast_row_style_scales_large_metrics() {
        let metrics = LargeMetrics::for_size(WidgetSize::from_dimensions(480, 320));
        let style = metrics.forecast_row_style();

        assert_eq!(style.label_size, 16);
        assert_eq!(style.icon_size, 26.666_668);
        assert_eq!(style.temperature_size, 21);
        assert_eq!(style.temperature_cell_width, 53.333_336);
        assert_eq!(style.bar_width, 93.333_336);
        assert_eq!(style.bar_height, 10.666_667);
    }

    #[test]
    fn bmm101_short_large_height_tightens_vertical_gaps() {
        let metrics = LargeMetrics::for_size(WidgetSize::from_dimensions(480, 320));

        assert_eq!(metrics.root_gap, 10.666_667);
        assert_eq!(metrics.current_gap, 4.0);
        assert_eq!(metrics.compact_icon_text_gap, 36.0);
    }

    #[test]
    fn canonical_large_keeps_authored_vertical_gaps() {
        let metrics = LargeMetrics::for_size(WidgetSize::from_dimensions(638, 480));

        assert_eq!(metrics.root_gap, 16.0);
        assert_eq!(metrics.current_gap, 8.0);
    }

    #[test]
    fn bmm101_current_condition_moves_right_of_icon() {
        let size = WidgetSize::from_dimensions(480, 320);
        init_test_registrars();
        let params = Params {
            location: "Prague".to_string(),
            time_zone: TimeZone::System,
        };
        let node = large(&weather(), &params, size);

        let Node::Column(_, root_children) = node else {
            panic!("BUG: Large weather root must be a column");
        };
        let Some(Node::Row(_, today_children)) = root_children.get(1) else {
            panic!("BUG: Large weather today block must be a row");
        };
        let Some(Node::Row(current_props, current_children)) = today_children.first() else {
            panic!("BUG: short Large current block must be a compact row");
        };
        let Some(Node::Canvas { .. }) = current_children.first() else {
            panic!("BUG: compact current block must keep icon first");
        };
        let Some(Node::Column(_, text_children)) = current_children.get(1) else {
            panic!("BUG: compact current block must place temperature and condition after icon");
        };

        assert_eq!(current_props.gap, 36.0);
        assert_eq!(text_children.len(), 2);
        assert!(matches!(text_children[0], Node::Paragraph { .. }));
        assert!(matches!(text_children[1], Node::Paragraph { .. }));
    }

    #[test]
    fn canonical_large_keeps_current_condition_stacked_below_icon() {
        let size = WidgetSize::from_dimensions(638, 480);
        init_test_registrars();
        let params = Params {
            location: "Prague".to_string(),
            time_zone: TimeZone::System,
        };
        let node = large(&weather(), &params, size);

        let Node::Column(_, root_children) = node else {
            panic!("BUG: Large weather root must be a column");
        };
        let Some(Node::Row(_, today_children)) = root_children.get(1) else {
            panic!("BUG: Large weather today block must be a row");
        };
        let Some(Node::Column(_, current_children)) = today_children.first() else {
            panic!("BUG: canonical Large current block must stay stacked");
        };

        assert_eq!(current_children.len(), 3);
        assert!(matches!(current_children[0], Node::Canvas { .. }));
        assert!(matches!(current_children[1], Node::Paragraph { .. }));
        assert!(matches!(current_children[2], Node::Paragraph { .. }));
    }

    #[test]
    fn bmm101_location_uses_full_non_wrapping_row() {
        let size = WidgetSize::from_dimensions(480, 320);
        init_test_registrars();
        let metrics = LargeMetrics::for_size(size);
        let params = Params {
            location: "Prague".to_string(),
            time_zone: TimeZone::System,
        };
        let node = large(&weather(), &params, size);

        let Node::Column(_, children) = node else {
            panic!("BUG: Large weather root must be a column");
        };
        let Some(Node::Paragraph {
            base_style, spans, ..
        }) = children.first()
        else {
            panic!("BUG: Large weather location must be the root column's first child");
        };

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, "Prague, Czech Republic");
        assert_eq!(base_style.text_overflow, TextOverflow::Clip);
        assert_eq!(base_style.max_width, metrics.location_max_width(size));
        assert_eq!(base_style.size, metrics.location_font_size);
    }

    fn weather() -> Weather {
        Weather {
            location: Location {
                display_name: "Prague, Czech Republic".to_string(),
                timezone: "Europe/Prague".to_string(),
            },
            current: Some(Current {
                temperature: DegreeCelsius(24.0),
                weather_code: 1,
                wind_speed: Availability::Unavailable,
                wind_direction: Availability::Unavailable,
                is_day: true,
            }),
            hourly: None,
            daily: Some(Daily {
                today_index: 0,
                today_sunrise: "2026-06-05T05:02:00+02:00".to_string(),
                today_sunset: "2026-06-05T21:04:00+02:00".to_string(),
                days: vec![
                    DayForecast {
                        time_rfc3339: "2026-06-05T12:00:00+02:00".to_string(),
                        weather_code: 1,
                        min: DegreeCelsius(16.0),
                        max: DegreeCelsius(27.0),
                    },
                    DayForecast {
                        time_rfc3339: "2026-06-06T12:00:00+02:00".to_string(),
                        weather_code: 0,
                        min: DegreeCelsius(17.0),
                        max: DegreeCelsius(29.0),
                    },
                ],
            }),
        }
    }
}

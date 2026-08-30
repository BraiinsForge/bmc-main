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

//! The sparkline view: header row on top, a canvas below drawing the
//! bottom-anchored sparkline with the tile-centered price over it.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "the shared render module's SDK imports and palette"
    )
)]
use super::*;
use prices::chart;

const CHART_STROKE: f32 = 2.0;
const CHART_INSET: f32 = 2.0;
/// The sparkline band fills the bottom fraction of the tile, the price number
/// floating over it — matching the legacy bitcoin ticker's `0.6 × height` graph.
const CHART_HEIGHT_FRACTION: f32 = 0.6;

/// The loaded view: header on top, sparkline + price on a canvas below.
#[must_use]
pub fn series_view(series: &Series, symbol: &str, period_label: &str, ws: WidgetSize) -> Node {
    let band = band_for(ws.variant).scaled(ws.fit());
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport dimensions are <= 1280, exact in f32"
    )]
    let (w, h) = (ws.width as f32, ws.height as f32);
    let trend = if series.is_positive() {
        TREND_UP
    } else {
        TREND_DOWN
    };
    let alpha = if series.is_closed_marked(symbol) {
        CLOSED_ALPHA
    } else {
        1.0
    };

    let header_h = header_height(&band);
    let canvas_h = (h - header_h).max(0.0);
    let header = super::header_row(series, symbol, period_label, &band, trend, alpha, header_h);

    // ── chart + price canvas ────────────────────────────────────────────
    let mut draws = Vec::new();
    let chart_height = (h * CHART_HEIGHT_FRACTION).min(canvas_h);
    let raw_line = chart::series_points(&series.sparkline_series(), w, chart_height, CHART_INSET);
    if raw_line.len() >= 2 {
        let y0 = canvas_h - chart_height;
        let line: Vec<(f32, f32)> = raw_line.iter().map(|&(x, y)| (x, y + y0)).collect();
        let mut area = line.clone();
        area.push((w, canvas_h));
        area.push((0.0, canvas_h));
        draws.push(fill!(
            area,
            linear: (
                trend.with_alpha(CHART_FILL_TOP_ALPHA * alpha),
                trend.with_alpha(CHART_FILL_BOTTOM_ALPHA * alpha)
            )
        ));
        draws.push(path!(line, stroke: CHART_STROKE, color: trend.with_alpha(alpha)));
    }
    // Above 100 000 a price loses its decimals but its digits grow without bound
    // (BTC-KRW reaches nine), so tile width runs out before font size does.
    let price_center_y = h / 2.0 - header_h;
    #[expect(
        clippy::cast_precision_loss,
        reason = "font sizes are small integers, exact in f32"
    )]
    let price_box_h = band.price_font as f32 * PRICE_BOX_LINES;
    draws.push(Draw::autofit_text(
        band.edge_padding,
        price_center_y - price_box_h / 2.0,
        (w - band.edge_padding * 2.0).max(1.0),
        price_box_h,
        super::price_text(symbol, series.current),
        style!(
            size: band.price_font,
            weight: FontWeight::BOLD,
            color: PRICE_COLOR,
            align: TextAlign::Center,
            valign: VerticalAlign::Center,
        ),
    ));

    col(
        props!(background: BACKGROUND, width: w, height: h),
        [header, canvas(props!(width: w, height: canvas_h), draws)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prices::candle::CandleBar;

    fn series() -> Series {
        Series {
            bars: vec![CandleBar {
                t_secs: 0,
                open: 90.0,
                high: 105.0,
                low: 85.0,
                close: 100.0,
                volume: None,
            }],
            current: 120.0,
            change_pct: 100.0 / 3.0,
            quote_currency: Some("USD".to_owned()),
            market_open: true,
        }
    }

    fn canvas_draws(node: Node) -> Vec<Draw> {
        let Node::Column(_, children) = node else {
            panic!("BUG: sparkline root must be a column");
        };
        let Some(Node::Canvas { draws, .. }) = children.into_iter().nth(1) else {
            panic!("BUG: sparkline chart must follow the header");
        };
        draws
    }

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport and font sizes are small integers, exact in f32"
    )]
    fn the_price_shrinks_to_the_tile_rather_than_being_sliced() {
        let ws = WidgetSize::from_dimensions(1_280, 480);
        let draws = canvas_draws(series_view(&series(), "BTC-USD", "1D", ws));
        let band = band_for(ws.variant).scaled(ws.fit());
        let boxes: Vec<_> = draws
            .iter()
            .filter_map(|draw| {
                let Draw::AutofitText {
                    box_width,
                    box_height,
                    style,
                    ..
                } = draw
                else {
                    return None;
                };
                Some((*box_width, *box_height, style.size))
            })
            .collect();

        let [(box_width, box_height, size)] = boxes[..] else {
            panic!("BUG: the sparkline must autofit exactly one price");
        };
        assert_eq!(size, band.price_font);
        assert!(
            (box_width - (ws.width as f32 - band.edge_padding * 2.0)).abs() < f32::EPSILON,
            "the price spans the tile between its edge paddings before it shrinks"
        );
        let one_line = band.price_font as f32 * 1.4;
        assert!(
            box_height >= one_line && box_height < one_line * 2.0,
            "a second authored-size line must not fit in the autofit box"
        );
    }
}

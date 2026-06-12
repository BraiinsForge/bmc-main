// Copyright (C) 2026  Braiins Systems s.r.o.

//! The sparkline view: header row on top, a canvas below drawing the
//! bottom-anchored sparkline with the tile-centered price over it.

#[expect(
    clippy::wildcard_imports,
    reason = "the shared render module's SDK imports and palette"
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
    let is_index = symbol.starts_with('^');
    let alpha = if !series.market_open && !is_index {
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
    let raw_line = chart::series_points(&series.closes(), w, chart_height, CHART_INSET);
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
    draws.push(Draw::text(
        w / 2.0,
        h / 2.0 - header_h,
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

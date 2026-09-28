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

//! The wasm render path: the per-size ticker grid (two columns at Full, a
//! single column otherwise), each row's symbol/company, optional
//! sparkline, and price/change, with hairline dividers between cells.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "widget render uses many SDK exports and macros"
    )
)]
use bmc_wasm_sdk::*;

use crate::layout::{Band, band_for};
use crate::model::{RowState, TickerRow};
use prices::chart;
use prices::closed_market::{CLOSED_CHART_ALPHA, pause_marker};
use prices::format::{MIN_PRICE, PricePrecision, change_text, price_precision};

const BACKGROUND: Color = BLACK;
const PRIMARY: Color = Color::from_rgb(0xf4, 0xf4, 0xf4);
const SECONDARY: Color = Color::from_rgb(0xc6, 0xc6, 0xc6);
const TREND_UP: Color = Color::from_rgb(0x42, 0xbe, 0x65);
const TREND_DOWN: Color = Color::from_rgb(0xfa, 0x4d, 0x56);
const ERROR: Color = Color::from_rgb(0xfa, 0x4d, 0x56);
const BORDER: Color = Color::from_rgb(0x52, 0x52, 0x52);
const BADGE_BG_ALPHA: f32 = 0.15;
const CHART_STROKE: f32 = 2.0;
const CHART_INSET: f32 = 2.0;
const CHART_FILL_TOP_ALPHA: f32 = 0.15;
const CHART_FILL_BOTTOM_ALPHA: f32 = 0.02;
const ERROR_ROW_ALPHA: f32 = 0.6;
const CLOSED_MARKER_SCALE: f32 = 0.75;

fn fixed_width(width: f32) -> Node {
    col(props!(width: width), Vec::<Node>::new())
}

fn h_divider() -> Node {
    col(props!(height: 1.0, background: BORDER), Vec::<Node>::new())
}

fn v_divider() -> Node {
    col(props!(width: 1.0, background: BORDER), Vec::<Node>::new())
}

/// One row's content, split across the list's columns.
struct Cells {
    name: Node,
    chart: Node,
    price: Node,
}

fn empty_cells(band: &Band) -> Cells {
    Cells {
        name: col(props!(flex: 1.0), Vec::<Node>::new()),
        chart: fixed_width(band.chart_width),
        price: col(props!(), Vec::<Node>::new()),
    }
}

fn closed_marker(band: &Band) -> Node {
    #[expect(
        clippy::cast_precision_loss,
        reason = "scaled font sizes are small, exact in f32"
    )]
    let diameter = scale_font(band.symbol_font, CLOSED_MARKER_SCALE) as f32;
    let draws = pause_marker(diameter, SECONDARY, BACKGROUND);
    canvas(props!(width: diameter, height: diameter), draws)
}

/// `symbol` line, with a pause marker after it when the market is closed
/// and, when the row is stale, a badge aging from its last good load.
fn symbol_line(
    symbol: &str,
    color: Color,
    band: &Band,
    closed: bool,
    stale: Option<SystemTime>,
) -> Node {
    let mut children = vec![text(
        symbol,
        style!(size: band.symbol_font, weight: FontWeight::BOLD, color: color),
    )];
    if closed {
        children.push(fixed_width(band.row_gap));
        children.push(closed_marker(band));
    }
    if let Some(anchor) = stale {
        children.push(fixed_width(band.row_gap));
        children.push(stale_badge(band, anchor));
    }
    row(props!(cross_align: CrossAlign::Center), children)
}

fn sparkline(series: &[f64], trend: Color, closed: bool, band: &Band) -> Node {
    let (w, h) = (band.chart_width, band.chart_height);
    let color = if closed { SECONDARY } else { trend };
    let alpha = if closed { CLOSED_CHART_ALPHA } else { 1.0 };
    let line = chart::series_points(series, w, h, CHART_INSET);
    if line.len() < 2 {
        return fixed_width(w);
    }
    let mut area = line.clone();
    area.push((w, h));
    area.push((0.0, h));
    canvas(
        props!(width: w, height: h),
        [
            fill!(
                area,
                linear: (
                    color.with_alpha(CHART_FILL_TOP_ALPHA * alpha),
                    color.with_alpha(CHART_FILL_BOTTOM_ALPHA * alpha)
                )
            ),
            path!(line, stroke: CHART_STROKE, color: color.with_alpha(alpha)),
        ],
    )
}

fn badge_node(text_str: String, trend: Color, band: &Band) -> Node {
    row(
        props!(background: trend.with_alpha(BADGE_BG_ALPHA), padding: band.badge_padding),
        [text(
            text_str,
            style!(size: band.change_font, weight: FontWeight::BOLD, color: trend),
        )],
    )
}

fn right_col(price_str: String, change_str: String, trend: Color, band: &Band) -> Node {
    col(
        props!(cross_align: CrossAlign::End, gap: band.row_gap),
        [
            text(
                price_str,
                style!(size: band.price_font, weight: FontWeight::BOLD, color: PRIMARY, align: TextAlign::Right),
            ),
            badge_node(change_str, trend, band),
        ],
    )
}

fn resolved_cells(
    row_data: &TickerRow,
    name: Option<&str>,
    stale: Option<SystemTime>,
    band: &Band,
) -> Cells {
    let trend = if row_data.is_positive() {
        TREND_UP
    } else {
        TREND_DOWN
    };
    let closed = row_data.is_closed_marked();
    let price = match price_precision(&row_data.symbol, row_data.price) {
        PricePrecision::Fraction(digits) => format_number!(row_data.price, digits),
        PricePrecision::BelowMin => {
            let mut out = String::from("<");
            out.push_str(&format_number!(MIN_PRICE, 6));
            out
        }
    };
    Cells {
        name: col(
            props!(flex: 1.0, cross_align: CrossAlign::Start, gap: band.row_gap),
            [
                symbol_line(&row_data.symbol, PRIMARY, band, closed, stale),
                text(
                    name.unwrap_or_default(),
                    style!(size: band.company_font, color: SECONDARY, text_overflow: TextOverflow::Ellipsis),
                ),
            ],
        ),
        chart: if band.show_sparkline {
            sparkline(&row_data.series, trend, closed, band)
        } else {
            fixed_width(0.0)
        },
        price: right_col(price, change_text(row_data.change_pct), trend, band),
    }
}

/// Per-row counterpart to the SDK's `with_stale_overlay` pill, rendered inline
/// after the symbol: a list polls each row separately, so a single pill floated
/// over the root could not say *which* rows are holding an old series.
///
/// It carries the SDK pill's Carbon Warning tokens rather than a palette of its
/// own, but builds its own chrome off the band — `Node::Tag`'s padding and icon
/// are fixed host-side constants, which at a 0.67 `fit` would leave a badge
/// wider than the sparkline sitting beside a 21px symbol. `band.stale_label`
/// adds the age; bands too narrow for it show the icon alone.
fn stale_badge(band: &Band, anchor: SystemTime) -> Node {
    let mut children = vec![canvas(
        props!(width: band.stale_icon, height: band.stale_icon),
        [Draw::svg_builtin(
            0.0,
            0.0,
            band.stale_icon,
            band.stale_icon,
            ICON_WARNING,
            ORANGE_40,
        )],
    )];
    if band.stale_label {
        children.push(relative_time_live(
            anchor,
            // Bare "5m", not the pill's "Last refresh 5m ago" — a row has room
            // for a magnitude, not a sentence.
            RelTimeFormat {
                length: RelTimeLength::Short,
                segments: RelTimeSegments::Single,
            },
            // An age only counts up; never "in …" on a clock step back.
            RelTimeClamp::ElapsedOnly,
            TextStyle {
                size: band.stale_font,
                weight: FontWeight::BOLD,
                color: ORANGE_40,
                ..TextStyle::default()
            },
        ));
    }
    row(
        props!(
            background: GRAY_100,
            padding: band.badge_padding,
            gap: band.badge_padding,
            cross_align: CrossAlign::Center
        ),
        children,
    )
}

/// A placeholder row: symbol (error-colored for not-found, gray otherwise) +
/// a short status, price `N/A`, and the whole row dimmed to 0.6.
fn placeholder_cells(symbol: &str, status: &str, symbol_color: Color, band: &Band) -> Cells {
    let sym = symbol_color.with_alpha(ERROR_ROW_ALPHA);
    let muted = SECONDARY.with_alpha(ERROR_ROW_ALPHA);
    Cells {
        name: col(
            props!(flex: 1.0, cross_align: CrossAlign::Start, gap: band.row_gap),
            [
                symbol_line(symbol, sym, band, false, None),
                text(status, style!(size: band.company_font, color: muted)),
            ],
        ),
        chart: fixed_width(band.chart_width),
        price: col(
            props!(cross_align: CrossAlign::End),
            [text(
                "N/A",
                style!(size: band.price_font, weight: FontWeight::BOLD, color: muted, align: TextAlign::Right),
            )],
        ),
    }
}

fn slot(
    index: usize,
    symbols: &[String],
    states: &[RowState],
    names: &[Option<String>],
    stale: &[Option<SystemTime>],
    band: &Band,
) -> Cells {
    let Some(symbol) = symbols.get(index) else {
        return empty_cells(band);
    };
    let Some(row_state) = states.get(index) else {
        return empty_cells(band);
    };
    match row_state {
        RowState::Resolved { data } => resolved_cells(
            data,
            names.get(index).and_then(Option::as_deref),
            stale.get(index).copied().flatten(),
            band,
        ),
        RowState::InputError { .. } => placeholder_cells(symbol, "Not found", ERROR, band),
        RowState::NoData { market_closed } => {
            let text = if *market_closed { "Closed" } else { "No data" };
            placeholder_cells(symbol, text, SECONDARY, band)
        }
        RowState::Failed => placeholder_cells(symbol, "Unavailable", SECONDARY, band),
        RowState::Loading => placeholder_cells(symbol, "Loading\u{2026}", SECONDARY, band),
    }
}

/// A list laid out a column at a time — names, charts, prices — so every chart starts
/// where the widest price leaves room, not against its own row's price.
fn list(rows: Vec<Cells>, band: &Band) -> Node {
    let pad = band.row_padding;
    let mut names = Vec::with_capacity(2 * rows.len());
    let mut charts = Vec::with_capacity(2 * rows.len());
    let mut prices = Vec::with_capacity(2 * rows.len());
    for (index, cells) in rows.into_iter().enumerate() {
        if index > 0 {
            names.push(h_divider());
            charts.push(h_divider());
            prices.push(h_divider());
        }
        names.push(row(
            props!(flex: 1.0, cross_align: CrossAlign::Center),
            [fixed_width(pad), cells.name],
        ));
        charts.push(row(
            props!(flex: 1.0, cross_align: CrossAlign::Center),
            [fixed_width(pad), cells.chart, fixed_width(pad)],
        ));
        prices.push(row(
            props!(flex: 1.0, cross_align: CrossAlign::Center, justify_content: Justify::End),
            [cells.price, fixed_width(pad)],
        ));
    }
    let mut columns = vec![col(props!(flex: 1.0), names)];
    if band.show_sparkline {
        columns.push(col(props!(), charts));
    }
    columns.push(col(props!(), prices));
    row(props!(flex: 1.0), columns)
}

/// The full grid for the current size.
#[must_use]
pub fn view(
    symbols: &[String],
    states: &[RowState],
    names: &[Option<String>],
    stale: &[Option<SystemTime>],
    ws: WidgetSize,
) -> Node {
    // Only an empty list collapses to a message. Rows that all failed keep
    // their placeholders: each names its symbol and why it is missing.
    if symbols.is_empty() {
        return message_view("No symbols provided", ws);
    }
    let band = band_for(ws.variant).scaled(ws.fit());
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport dimensions are <= 1280, exact in f32"
    )]
    let (w, h) = (ws.width as f32, ws.height as f32);

    let rows: Vec<Cells> = (0..band.rows)
        .map(|index| slot(index, symbols, states, names, stale, &band))
        .collect();
    let body = if band.columns == 2 {
        // Filled left to right, so the first two symbols share the top row.
        let (left, right): (Vec<_>, Vec<_>) = rows
            .into_iter()
            .enumerate()
            .partition(|(index, _)| index % 2 == 0);
        let half = |half: Vec<(usize, Cells)>| {
            let rows = half.into_iter().map(|(_, cells)| cells).collect();
            list(rows, &band)
        };
        // Both halves on whole pixels: taffy 0.9 rounds a node's location
        // against its parent but its width against the absolute edge,
        // so a half that starts on a half pixel opens a one-pixel gap
        // in its rules where its columns meet.
        let left_width = ((w - 1.0) / 2.0).floor();
        row(
            props!(flex: 1.0),
            [
                col(props!(width: left_width), [half(left)]),
                v_divider(),
                half(right),
            ],
        )
    } else {
        list(rows, &band)
    };
    col(props!(background: BACKGROUND, width: w, height: h), [body])
}

fn message_view(message: &str, ws: WidgetSize) -> Node {
    let band = band_for(ws.variant).scaled(ws.fit());
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport dimensions are <= 1280, exact in f32"
    )]
    let (w, h) = (ws.width as f32, ws.height as f32);
    col(
        props!(background: BACKGROUND, width: w, height: h, cross_align: CrossAlign::Center),
        [
            spacer(1.0),
            text(
                message,
                style!(size: band.company_font, color: SECONDARY, align: TextAlign::Center),
            ),
            spacer(1.0),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    /// Every paragraph the tree would draw, in tree order.
    fn texts(node: &Node) -> Vec<String> {
        let mut out = Vec::new();
        collect_texts(node, &mut out);
        out
    }

    fn collect_texts(node: &Node, out: &mut Vec<String>) {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                for child in children {
                    collect_texts(child, out);
                }
            }
            Node::Paragraph { spans, .. } => {
                out.push(spans.iter().map(|span| span.text.as_str()).collect());
            }
            _ => {}
        }
    }

    fn view_of(list: &fixtures::List, size: WidgetSize) -> Node {
        view(&list.symbols, &list.states, &list.names, &list.stale, size)
    }

    #[test]
    fn an_empty_list_reads_as_a_message_not_blank_rows() {
        let size = WidgetSize::from_dimensions(638, 480);
        assert_eq!(
            texts(&view_of(&fixtures::no_symbols(), size)),
            ["No symbols provided"]
        );
    }

    #[test]
    fn a_list_whose_rows_all_failed_keeps_each_placeholder() {
        let size = WidgetSize::from_dimensions(638, 480);
        let texts = texts(&view_of(&fixtures::failed(), size));
        assert_eq!(
            texts.iter().filter(|text| *text == "Unavailable").count(),
            crate::layout::size_capacity(SizeVariant::Large),
            "every seated row names why it is missing: {texts:?}"
        );
    }

    #[test]
    fn full_fills_left_to_right_so_the_first_two_symbols_share_the_top_row() {
        let size = WidgetSize::from_dimensions(1_280, 480);
        let Node::Column(_, body) = view_of(&fixtures::healthy(), size) else {
            panic!("BUG: the view is a column");
        };
        let Some(Node::Row(_, halves)) = body.first() else {
            panic!("BUG: Full lays its halves out in a row");
        };
        let lead = |half: &Node| texts(half).first().cloned();
        assert_eq!(lead(&halves[0]).as_deref(), Some("NVDA"));
        assert_eq!(lead(&halves[2]).as_deref(), Some("AAPL"));
    }

    fn sparkline_stroke_color(node: Node) -> Color {
        let Node::Canvas { draws, .. } = node else {
            panic!("BUG: a two-point sparkline is a canvas");
        };
        draws
            .iter()
            .find_map(|draw| {
                let Draw::Path {
                    paint: PathPaint::Stroke { color, .. },
                    ..
                } = draw
                else {
                    return None;
                };
                Some(*color)
            })
            .expect("BUG: the sparkline must contain a stroked path")
    }

    #[test]
    fn a_closed_sparkline_uses_the_grey_deck_alpha() {
        let band = band_for(SizeVariant::Full);
        let series = [1.0, 2.0];
        assert_eq!(
            sparkline_stroke_color(sparkline(&series, TREND_UP, false, &band)),
            TREND_UP
        );
        assert_eq!(
            sparkline_stroke_color(sparkline(&series, TREND_UP, true, &band)),
            SECONDARY.with_alpha(CLOSED_CHART_ALPHA)
        );
    }

    #[test]
    fn a_company_name_reaches_the_renderer_whole_to_be_ellipsized() {
        use prices::candle::{CandleBar, Candles};

        const NAME: &str = "Grayscale Bitcoin Mini Trust ETF, longer than any row seats";
        let bar = |t_secs| CandleBar {
            t_secs,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: None,
        };
        let candles = Candles {
            bars: vec![bar(0), bar(3_600)],
            quote_currency: None,
        };
        let row_data = TickerRow::from_candles("BTC", &candles).expect("BUG: candles build a row");

        let cells = resolved_cells(&row_data, Some(NAME), None, &band_for(SizeVariant::Small));
        let Node::Column(_, left) = cells.name else {
            panic!("BUG: the name cell is a column");
        };
        let Some(Node::Paragraph {
            base_style, spans, ..
        }) = left.get(1)
        else {
            panic!("BUG: the company name follows the symbol line");
        };
        assert_eq!(spans[0].text, NAME);
        assert_eq!(base_style.text_overflow, TextOverflow::Ellipsis);
    }
}

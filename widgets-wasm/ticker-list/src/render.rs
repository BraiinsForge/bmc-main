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
//! BMM101 draws a frame of its own, in [`bmm101`].

mod bmm101;

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "widget render uses many SDK exports and macros"
    )
)]
use bmc_wasm_sdk::*;

use crate::layout::{Band, SizeBucket, band_for, size_bucket};
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
/// The SDK stale pill's space between its icon and label.
const STALE_ICON_GAP: f32 = 8.0;
const NO_SYMBOLS: &str = "No symbols provided";

fn fixed_width(width: f32) -> Node {
    col(props!(width: width), Vec::<Node>::new())
}

fn rule_line(color: Color) -> Node {
    col(props!(height: 1.0, background: color), Vec::<Node>::new())
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

/// What one row slot holds, before a layout styles it.
#[derive(Clone, Copy)]
enum Slot<'a> {
    Empty,
    Resolved {
        data: &'a TickerRow,
        name: Option<&'a str>,
        stale: Option<SystemTime>,
    },
    Placeholder {
        symbol: &'a str,
        status: &'static str,
        not_found: bool,
    },
}

fn slot<'a>(
    index: usize,
    symbols: &'a [String],
    states: &'a [RowState],
    names: &'a [Option<String>],
    stale: &[Option<SystemTime>],
) -> Slot<'a> {
    let (Some(symbol), Some(row_state)) = (symbols.get(index), states.get(index)) else {
        return Slot::Empty;
    };
    let placeholder = |status, not_found| Slot::Placeholder {
        symbol,
        status,
        not_found,
    };
    match row_state {
        RowState::Resolved { data } => Slot::Resolved {
            data,
            name: names.get(index).and_then(Option::as_deref),
            stale: stale.get(index).copied().flatten(),
        },
        RowState::InputError { .. } => placeholder("Not found", true),
        RowState::NoData { market_closed } => {
            placeholder(if *market_closed { "Closed" } else { "No data" }, false)
        }
        RowState::Failed => placeholder("Unavailable", false),
        RowState::Loading => placeholder("Loading\u{2026}", false),
    }
}

fn price_text(row_data: &TickerRow) -> String {
    match price_precision(&row_data.symbol, row_data.price) {
        PricePrecision::Fraction(digits) => format_number!(row_data.price, digits),
        PricePrecision::BelowMin => {
            let mut out = String::from("<");
            out.push_str(&format_number!(MIN_PRICE, 6));
            out
        }
    }
}

fn closed_marker(band: &Band, color: Color) -> Node {
    let diameter = band.marker_size;
    let draws = pause_marker(diameter, color, BACKGROUND);
    canvas(props!(width: diameter, height: diameter), draws)
}

/// The `symbol` node, with a pause marker in `marker` after it when the market is closed.
fn symbol_line(symbol: Node, band: &Band, marker: Color, closed: bool) -> Node {
    let mut children = vec![symbol];
    if closed {
        children.push(fixed_width(band.row_gap));
        children.push(closed_marker(band, marker));
    }
    row(props!(cross_align: CrossAlign::Center), children)
}

/// Stands in for the company name while a row is stale:
/// the SDK pill's warning icon and the age of the last good load, in the name's type.
/// Per row, because the SDK's `with_stale_overlay` floats one pill over the whole root
/// and so cannot say *which* rows hold an old series.
fn stale_line(anchor: SystemTime, name: TextStyle, band: &Band) -> Node {
    let icon = band.marker_size;
    row(
        props!(gap: STALE_ICON_GAP, cross_align: CrossAlign::Center),
        [
            canvas(
                props!(width: icon, height: icon),
                [Draw::svg_builtin(
                    0.0,
                    0.0,
                    icon,
                    icon,
                    ICON_WARNING,
                    ORANGE_40,
                )],
            ),
            relative_time_live(
                anchor,
                // "12m ago", not the pill's "Last refresh 12m ago":
                // BMM101's name column is narrower than the sentence.
                RelTimeFormat {
                    length: RelTimeLength::Short,
                    segments: RelTimeSegments::Single,
                },
                // An age only counts up; never "in …" on a clock step back.
                RelTimeClamp::ElapsedOnly,
                TextStyle {
                    color: ORANGE_40,
                    ..name
                },
            ),
        ],
    )
}

/// How a sparkline strokes its line and fills the area under it.
struct Paint {
    line: Color,
    fill_top: Color,
    fill_bottom: Color,
    stroke: f32,
}

/// A closed market greys the line, a live one takes the trend's colour.
fn deck_paint(trend: Color, closed: bool) -> Paint {
    let color = if closed { SECONDARY } else { trend };
    let alpha = if closed { CLOSED_CHART_ALPHA } else { 1.0 };
    Paint {
        line: color.with_alpha(alpha),
        fill_top: color.with_alpha(CHART_FILL_TOP_ALPHA * alpha),
        fill_bottom: color.with_alpha(CHART_FILL_BOTTOM_ALPHA * alpha),
        stroke: CHART_STROKE,
    }
}

fn sparkline(series: &[f64], paint: &Paint, band: &Band) -> Node {
    let (w, h) = (band.chart_width, band.chart_height);
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
            fill!(area, linear: (paint.fill_top, paint.fill_bottom)),
            path!(line, stroke: paint.stroke, color: paint.line),
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
    let name_style =
        style!(size: band.company_font, color: SECONDARY, text_overflow: TextOverflow::Ellipsis);
    Cells {
        name: col(
            props!(flex: 1.0, gap: band.row_gap),
            [
                symbol_line(
                    deck_symbol(&row_data.symbol, PRIMARY, band),
                    band,
                    SECONDARY,
                    closed,
                ),
                match stale {
                    Some(anchor) => stale_line(anchor, name_style.0, band),
                    None => text(name.unwrap_or_default(), name_style),
                },
            ],
        ),
        chart: if band.show_sparkline {
            sparkline(&row_data.series, &deck_paint(trend, closed), band)
        } else {
            fixed_width(0.0)
        },
        price: right_col(
            price_text(row_data),
            change_text(row_data.change_pct),
            trend,
            band,
        ),
    }
}

fn deck_symbol(symbol: &str, color: Color, band: &Band) -> Node {
    text(
        symbol,
        style!(size: band.symbol_font, weight: FontWeight::BOLD, color: color, text_overflow: TextOverflow::Ellipsis),
    )
}

/// A placeholder row: symbol (error-colored for not-found, gray otherwise) +
/// a short status, price `N/A`, and the whole row dimmed to 0.6.
fn placeholder_cells(symbol: &str, status: &str, symbol_color: Color, band: &Band) -> Cells {
    let sym = symbol_color.with_alpha(ERROR_ROW_ALPHA);
    let muted = SECONDARY.with_alpha(ERROR_ROW_ALPHA);
    Cells {
        name: col(
            props!(flex: 1.0, gap: band.row_gap),
            [
                symbol_line(deck_symbol(symbol, sym, band), band, SECONDARY, false),
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

fn deck_cells(slot: Slot, band: &Band) -> Cells {
    match slot {
        Slot::Empty => empty_cells(band),
        Slot::Resolved { data, name, stale } => resolved_cells(data, name, stale, band),
        Slot::Placeholder {
            symbol,
            status,
            not_found,
        } => placeholder_cells(
            symbol,
            status,
            if not_found { ERROR } else { SECONDARY },
            band,
        ),
    }
}

/// How a list spaces its columns and rules.
struct Grid {
    /// Space at the list's outer edges.
    edge: f32,
    /// Space on either side of the chart column, or between name and price without one.
    column_gap: f32,
    show_charts: bool,
    rule: Color,
    /// Space above and below each rule.
    rule_gap: f32,
    /// Every row this tall; `None` shares the list's height out evenly.
    row_height: Option<f32>,
    /// The price column is never narrower than this.
    price_min_width: Option<f32>,
}

impl Grid {
    fn deck(band: &Band) -> Self {
        Self {
            edge: band.row_padding,
            column_gap: band.row_padding,
            show_charts: band.show_sparkline,
            rule: BORDER,
            rule_gap: 0.0,
            row_height: None,
            price_min_width: None,
        }
    }

    fn cell(&self, justify: Justify, children: Vec<Node>) -> Node {
        match self.row_height {
            Some(height) => row(
                props!(height: height, cross_align: CrossAlign::Center, justify_content: justify),
                children,
            ),
            None => row(
                props!(flex: 1.0, cross_align: CrossAlign::Center, justify_content: justify),
                children,
            ),
        }
    }

    fn rule(&self) -> Node {
        if self.rule_gap > 0.0 {
            col(
                props!(height: 2.0 * self.rule_gap + 1.0, justify_content: Justify::Center),
                [rule_line(self.rule)],
            )
        } else {
            rule_line(self.rule)
        }
    }
}

/// A list laid out a column at a time — names, charts, prices — so every chart starts
/// where the widest price leaves room, not against its own row's price.
/// Without charts there is nothing to line up, so each row takes its own price's width.
fn list(rows: Vec<Cells>, grid: &Grid) -> Node {
    if !grid.show_charts {
        return row_list(rows, grid);
    }
    let mut names = Vec::with_capacity(2 * rows.len());
    let mut charts = Vec::with_capacity(2 * rows.len());
    let mut prices = Vec::with_capacity(2 * rows.len() + 1);
    for (index, cells) in rows.into_iter().enumerate() {
        if index > 0 {
            names.push(grid.rule());
            charts.push(grid.rule());
            prices.push(grid.rule());
        }
        names.push(grid.cell(Justify::Start, vec![fixed_width(grid.edge), cells.name]));
        charts.push(grid.cell(
            Justify::Start,
            vec![
                fixed_width(grid.column_gap),
                cells.chart,
                fixed_width(grid.column_gap),
            ],
        ));
        prices.push(grid.cell(Justify::End, vec![cells.price, fixed_width(grid.edge)]));
    }
    if let Some(width) = grid.price_min_width {
        prices.push(fixed_width(width));
    }
    row(
        props!(flex: 1.0),
        [
            col(props!(flex: 1.0), names),
            col(props!(), charts),
            col(props!(), prices),
        ],
    )
}

/// One row after another, each name giving way only to its own price.
fn row_list(rows: Vec<Cells>, grid: &Grid) -> Node {
    debug_assert!(
        grid.price_min_width.is_none(),
        "a row list has no shared price column to hold a minimum width"
    );
    let mut children = Vec::with_capacity(2 * rows.len());
    for (index, cells) in rows.into_iter().enumerate() {
        if index > 0 {
            children.push(grid.rule());
        }
        children.push(grid.cell(
            Justify::Start,
            vec![
                fixed_width(grid.edge),
                cells.name,
                fixed_width(grid.column_gap),
                cells.price,
                fixed_width(grid.edge),
            ],
        ));
    }
    col(props!(flex: 1.0), children)
}

/// The list for the current size: BMM101's own frame, or the Deck's grid.
#[must_use]
pub fn view(
    symbols: &[String],
    states: &[RowState],
    names: &[Option<String>],
    stale: &[Option<SystemTime>],
    ws: WidgetSize,
) -> Node {
    match size_bucket(ws.width, ws.height) {
        SizeBucket::Bmm101 => bmm101::view(symbols, states, names, stale, ws),
        SizeBucket::Full | SizeBucket::Large | SizeBucket::Medium | SizeBucket::Small => {
            deck_view(symbols, states, names, stale, ws)
        }
    }
}

fn deck_view(
    symbols: &[String],
    states: &[RowState],
    names: &[Option<String>],
    stale: &[Option<SystemTime>],
    ws: WidgetSize,
) -> Node {
    // Only an empty list collapses to a message. Rows that all failed keep
    // their placeholders: each names its symbol and why it is missing.
    if symbols.is_empty() {
        return message_view(NO_SYMBOLS, ws);
    }
    let band = band_for(ws.variant).scaled(ws.fit());
    let grid = Grid::deck(&band);
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport dimensions are <= 1280, exact in f32"
    )]
    let (w, h) = (ws.width as f32, ws.height as f32);

    let rows: Vec<Cells> = (0..band.rows)
        .map(|index| deck_cells(slot(index, symbols, states, names, stale), &band))
        .collect();
    let body = if band.columns == 2 {
        // Filled left to right, so the first two symbols share the top row.
        let (left, right): (Vec<_>, Vec<_>) = rows
            .into_iter()
            .enumerate()
            .partition(|(index, _)| index % 2 == 0);
        let half = |half: Vec<(usize, Cells)>| {
            let rows = half.into_iter().map(|(_, cells)| cells).collect();
            list(rows, &grid)
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
        list(rows, &grid)
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

    /// The style of the paragraph drawing `wanted`, anywhere in the tree.
    fn style_of(node: &Node, wanted: &str) -> Option<TextStyle> {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                children.iter().find_map(|child| style_of(child, wanted))
            }
            Node::Paragraph {
                base_style, spans, ..
            } if spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
                == wanted =>
            {
                Some(*base_style)
            }
            _ => None,
        }
    }

    fn bmm101() -> WidgetSize {
        WidgetSize::from_dimensions(480, 320)
    }

    #[test]
    fn the_bmm101_frame_reads_top_down_as_designed() {
        let texts = texts(&view_of(&fixtures::mixed(), bmm101()));
        assert_eq!(
            texts[..9],
            [
                "Financial Ticker List",
                "NVDA",
                "NVIDIA Corporation",
                "TSLA",
                "Closed",
                "NONEXS",
                "Not found",
                "AAPL",
                "Apple Inc.",
            ],
            "the header, then each row's symbol over its name or status: {texts:?}"
        );
        assert_eq!(
            texts.iter().filter(|text| *text == "N/A").count(),
            2,
            "the shut market and the unknown symbol carry no price: {texts:?}"
        );
    }

    #[test]
    fn a_long_symbol_and_name_are_cut_rather_than_wrapped() {
        let list = fixtures::extremes(fixtures::Extremes {
            long_strings: true,
            ..fixtures::Extremes::default()
        });
        for size in [bmm101(), WidgetSize::from_dimensions(638, 480)] {
            let view = view_of(&list, size);
            for text in [fixtures::LONG_SYMBOL, fixtures::LONGEST_NAME] {
                let style = style_of(&view, text).expect("BUG: the row draws its symbol and name");
                assert_eq!(
                    style.text_overflow,
                    TextOverflow::Ellipsis,
                    "{size:?}: {text}"
                );
            }
        }
    }

    fn count_ages(node: &Node) -> usize {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                children.iter().map(count_ages).sum()
            }
            Node::RelTime { .. } => 1,
            _ => 0,
        }
    }

    #[test]
    fn a_stale_row_shows_its_age_in_place_of_the_company_name() {
        for size in [bmm101(), WidgetSize::from_dimensions(638, 480)] {
            let view = view_of(&fixtures::stale(), size);
            let texts = texts(&view);
            assert!(
                !texts.iter().any(|text| text == "NVIDIA Corporation"),
                "{size:?}: {texts:?}"
            );
            assert_eq!(count_ages(&view), crate::layout::capacity(size), "{size:?}");
        }
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
            crate::layout::capacity(size),
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

    #[test]
    fn a_chartless_list_keeps_each_price_in_its_own_row() {
        let size = WidgetSize::from_dimensions(317, 238);
        let Node::Column(_, body) = view_of(&fixtures::healthy(), size) else {
            panic!("BUG: the view is a column");
        };
        let Some(Node::Column(_, rows)) = body.first() else {
            panic!("BUG: a chartless list stacks its rows in a column");
        };
        let first = texts(&rows[0]);
        assert!(
            first.iter().any(|text| text == "NVDA") && first.iter().any(|text| text == "+8.1%"),
            "the first row holds its symbol and its own change: {first:?}"
        );
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
            sparkline_stroke_color(sparkline(&series, &deck_paint(TREND_UP, false), &band)),
            TREND_UP
        );
        assert_eq!(
            sparkline_stroke_color(sparkline(&series, &deck_paint(TREND_UP, true), &band)),
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

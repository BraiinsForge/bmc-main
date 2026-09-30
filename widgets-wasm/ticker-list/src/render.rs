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
const NOT_AVAILABLE: &str = "N/A";
/// The thickness of a rule between rows and of Full's divider between halves.
const HAIRLINE: f32 = 1.0;

fn fixed_width(width: f32) -> Node {
    col(props!(width: width), Vec::<Node>::new())
}

fn rule_line(color: Color) -> Node {
    col(
        props!(height: HAIRLINE, background: color),
        Vec::<Node>::new(),
    )
}

/// A rule's height with `gap` of space above and below it.
fn rule_height(gap: f32) -> f32 {
    2.0 * gap + HAIRLINE
}

fn v_divider() -> Node {
    col(
        props!(width: HAIRLINE, background: BORDER),
        Vec::<Node>::new(),
    )
}

/// One row's content, split across the list's columns.
struct Cells {
    name: Node,
    chart: Node,
    price: Node,
}

/// Why a row has no price to show.
#[derive(Clone, Copy)]
enum Pending {
    Loading,
    Failed,
    NoData,
    Closed,
    NotFound,
}

impl Pending {
    fn label(self) -> &'static str {
        match self {
            Pending::Loading => "Loading\u{2026}",
            Pending::Failed => "Unavailable",
            Pending::NoData => "No data",
            Pending::Closed => "Closed",
            Pending::NotFound => "Not found",
        }
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
        reason: Pending,
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
    let placeholder = |reason| Slot::Placeholder { symbol, reason };
    match row_state {
        RowState::Resolved { data } => Slot::Resolved {
            data,
            name: names.get(index).and_then(Option::as_deref),
            stale: stale.get(index).copied().flatten(),
        },
        RowState::InputError { .. } => placeholder(Pending::NotFound),
        RowState::NoData {
            market_closed: true,
        } => placeholder(Pending::Closed),
        RowState::NoData {
            market_closed: false,
        } => placeholder(Pending::NoData),
        RowState::Failed => placeholder(Pending::Failed),
        RowState::Loading => placeholder(Pending::Loading),
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

fn closed_marker(diameter: f32, color: Color) -> Node {
    let draws = pause_marker(diameter, color, BACKGROUND);
    canvas(props!(width: diameter, height: diameter), draws)
}

/// The `symbol` node, with a pause marker after it when the market is closed.
fn symbol_line(row_style: &impl RowStyle, symbol: Node, closed: bool) -> Node {
    let mut children = vec![symbol];
    if closed {
        children.push(fixed_width(row_style.marker_gap()));
        children.push(closed_marker(
            row_style.marker_size(),
            row_style.secondary(),
        ));
    }
    row(props!(cross_align: CrossAlign::Center), children)
}

/// Stands in for the company name while a row is stale:
/// the SDK pill's warning icon and the age of the last good load, in the name's type.
/// Per row, because the SDK's `with_stale_overlay` floats one pill over the whole root
/// and so cannot say *which* rows hold an old series.
fn stale_line(anchor: SystemTime, name: TextStyle, icon: f32) -> Node {
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

fn sparkline(series: &[f64], paint: &Paint, (w, h): (f32, f32)) -> Node {
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

/// How a frame draws the pieces every row is made of, so the rows themselves are built once.
trait RowStyle {
    fn symbol(&self, symbol: &str, color: Color) -> Node;
    /// The type of the line under the symbol: a company name, why a price is missing,
    /// or how old a stale one is.
    fn sub_style(&self, color: Color) -> StyleResult;
    /// Seats a line under the symbol; a frame of fixed line boxes wraps it in one.
    fn sub_line(&self, line: Node) -> Node {
        line
    }
    fn price(&self, price: String, color: Color) -> Node;
    fn change(&self, change: String, rising: bool) -> Node;
    fn paint(&self, rising: bool, closed: bool) -> Paint;
    /// The symbol and the price.
    fn primary(&self) -> Color;
    /// The name, the pause marker and a placeholder's text.
    fn secondary(&self) -> Color;
    /// A placeholder's symbol, before the row dims.
    fn placeholder_color(&self, reason: Pending) -> Color;
    /// Between a symbol and its name, and between a price and its change.
    fn line_gap(&self) -> f32;
    fn marker_gap(&self) -> f32;
    /// The pause marker's diameter, and the stale warning's icon.
    fn marker_size(&self) -> f32;
    fn chart_size(&self) -> (f32, f32);
    fn show_chart(&self) -> bool;
}

/// The Deck's rows, sized by the band of the current frame.
struct Deck(Band);

fn trend(rising: bool) -> Color {
    if rising { TREND_UP } else { TREND_DOWN }
}

impl RowStyle for Deck {
    fn symbol(&self, symbol: &str, color: Color) -> Node {
        text(
            symbol,
            style!(size: self.0.symbol_font, weight: FontWeight::BOLD, color: color, text_overflow: TextOverflow::Ellipsis),
        )
    }

    fn sub_style(&self, color: Color) -> StyleResult {
        style!(size: self.0.company_font, color: color, text_overflow: TextOverflow::Ellipsis)
    }

    fn price(&self, price: String, color: Color) -> Node {
        text(
            price,
            style!(size: self.0.price_font, weight: FontWeight::BOLD, color: color, align: TextAlign::Right),
        )
    }

    fn change(&self, change: String, rising: bool) -> Node {
        let color = trend(rising);
        row(
            props!(background: color.with_alpha(BADGE_BG_ALPHA), padding: self.0.badge_padding),
            [text(
                change,
                style!(size: self.0.change_font, weight: FontWeight::BOLD, color: color),
            )],
        )
    }

    /// A closed market greys the line, a live one takes the trend's colour.
    fn paint(&self, rising: bool, closed: bool) -> Paint {
        let color = if closed { SECONDARY } else { trend(rising) };
        let alpha = if closed { CLOSED_CHART_ALPHA } else { 1.0 };
        Paint {
            line: color.with_alpha(alpha),
            fill_top: color.with_alpha(CHART_FILL_TOP_ALPHA * alpha),
            fill_bottom: color.with_alpha(CHART_FILL_BOTTOM_ALPHA * alpha),
            stroke: CHART_STROKE,
        }
    }

    fn primary(&self) -> Color {
        PRIMARY
    }

    fn secondary(&self) -> Color {
        SECONDARY
    }

    fn placeholder_color(&self, reason: Pending) -> Color {
        match reason {
            Pending::NotFound => ERROR,
            Pending::Loading | Pending::Failed | Pending::NoData | Pending::Closed => SECONDARY,
        }
    }

    fn line_gap(&self) -> f32 {
        self.0.row_gap
    }

    fn marker_gap(&self) -> f32 {
        self.0.row_gap
    }

    fn marker_size(&self) -> f32 {
        self.0.marker_size
    }

    fn chart_size(&self) -> (f32, f32) {
        (self.0.chart_width, self.0.chart_height)
    }

    fn show_chart(&self) -> bool {
        self.0.show_sparkline
    }
}

fn empty_cells(row_style: &impl RowStyle) -> Cells {
    Cells {
        name: col(props!(flex: 1.0), Vec::<Node>::new()),
        chart: fixed_width(row_style.chart_size().0),
        price: col(props!(), Vec::<Node>::new()),
    }
}

fn resolved_cells(
    row_style: &impl RowStyle,
    data: &TickerRow,
    name: Option<&str>,
    stale: Option<SystemTime>,
) -> Cells {
    let rising = data.is_positive();
    let closed = data.is_closed_marked();
    let sub = row_style.sub_style(row_style.secondary());
    Cells {
        name: col(
            props!(flex: 1.0, gap: row_style.line_gap()),
            [
                symbol_line(
                    row_style,
                    row_style.symbol(&data.symbol, row_style.primary()),
                    closed,
                ),
                row_style.sub_line(match stale {
                    Some(anchor) => stale_line(anchor, sub.0, row_style.marker_size()),
                    None => text(name.unwrap_or_default(), sub),
                }),
            ],
        ),
        chart: if row_style.show_chart() {
            sparkline(
                &data.series,
                &row_style.paint(rising, closed),
                row_style.chart_size(),
            )
        } else {
            fixed_width(0.0)
        },
        price: col(
            props!(cross_align: CrossAlign::End, gap: row_style.line_gap()),
            [
                row_style.price(price_text(data), row_style.primary()),
                row_style.change(change_text(data.change_pct), rising),
            ],
        ),
    }
}

/// A row without a price: its symbol, why the price is missing and `N/A`, all dimmed.
fn placeholder_cells(row_style: &impl RowStyle, symbol: &str, reason: Pending) -> Cells {
    let muted = row_style.secondary().with_alpha(ERROR_ROW_ALPHA);
    let symbol_color = row_style
        .placeholder_color(reason)
        .with_alpha(ERROR_ROW_ALPHA);
    Cells {
        name: col(
            props!(flex: 1.0, gap: row_style.line_gap()),
            [
                symbol_line(row_style, row_style.symbol(symbol, symbol_color), false),
                row_style.sub_line(text(reason.label(), row_style.sub_style(muted))),
            ],
        ),
        chart: fixed_width(row_style.chart_size().0),
        price: row_style.price(String::from(NOT_AVAILABLE), muted),
    }
}

fn row_cells(row_style: &impl RowStyle, slot: Slot) -> Cells {
    match slot {
        Slot::Empty => empty_cells(row_style),
        Slot::Resolved { data, name, stale } => resolved_cells(row_style, data, name, stale),
        Slot::Placeholder { symbol, reason } => placeholder_cells(row_style, symbol, reason),
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
                props!(height: rule_height(self.rule_gap), justify_content: Justify::Center),
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

    let row_style = Deck(band);
    let rows: Vec<Cells> = (0..band.rows)
        .map(|index| row_cells(&row_style, slot(index, symbols, states, names, stale)))
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
            texts.iter().filter(|text| *text == NOT_AVAILABLE).count(),
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
        let deck = Deck(band_for(SizeVariant::Full));
        let series = [1.0, 2.0];
        let stroke = |closed| {
            sparkline_stroke_color(sparkline(
                &series,
                &deck.paint(true, closed),
                deck.chart_size(),
            ))
        };
        assert_eq!(stroke(false), TREND_UP);
        assert_eq!(stroke(true), SECONDARY.with_alpha(CLOSED_CHART_ALPHA));
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

        let cells = resolved_cells(
            &Deck(band_for(SizeVariant::Small)),
            &row_data,
            Some(NAME),
            None,
        );
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

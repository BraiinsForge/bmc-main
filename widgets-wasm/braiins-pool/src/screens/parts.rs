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

//! Bucket-free fragments shared by the screens' per-size layouts:
//!  - design tokens
//!  - the header-left run and hero stat pairs
//!  - cards and stat blocks
//!  - the workers-by-state panel and payout meter
//!  - the unbound bind-hint body
//!
//! Every fragment takes its geometry from parameters; which variant a
//! frame gets is the layouts' decision.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;
use units::availability::Availability;

use crate::model::{SizeBucket, WorkerCounts};
use crate::screens::icons;

// Shared design tokens, mapped 1:1 to the Figma "Braiins Pool" frames.
// Metrics used by a single component live beside that component.

pub mod color {
    use bmc_wasm_sdk::{
        Color, GRAY_40, GRAY_60, GRAY_80, GRAY_90, GREEN_50, ORANGE_40, RED_50, TEAL_30, VIOLET_50,
        VIOLET_60, WHITE,
    };

    pub const BG: Color = Color::from_hex(0x09_09_09);
    /// Cards float on the background at the design's 70 % opacity.
    pub const CARD_BG: Color = Color::from_rgba(0x16, 0x16, 0x16, 179);
    pub const CARD_BORDER: Color = GRAY_90;
    pub const TEXT: Color = WHITE;
    pub const TEXT_MUTED: Color = GRAY_40;
    /// A "nothing there" callout; Carbon g100 `$text-placeholder`.
    pub const TEXT_ABSENT: Color = GRAY_60;
    pub const HASHRATE: Color = VIOLET_60;
    /// Hero values run one violet step lighter than the chart line.
    pub const HASHRATE_VALUE: Color = VIOLET_50;
    pub const WORKERS: Color = TEAL_30;
    pub const GRID: Color = GRAY_80;
    pub const STATE_OK: Color = GREEN_50;
    pub const STATE_LOW: Color = ORANGE_40;
    pub const STATE_OFF: Color = RED_50;
    pub const ERROR: Color = RED_50;
    pub const METER_FILL: Color = GREEN_50;
    pub const METER_TRACK: Color = GRAY_90;
    pub const SKELETON: Color = bmc_wasm_sdk::skeleton::ELEMENT_ON_DARK;
    /// The compact hero's `·`; no palette token carries #525252.
    pub const SEPARATOR: Color = Color::from_hex(0x52_52_52);
}

pub mod font {
    pub const TITLE: u32 = 24;
    /// Labels, subs, and legend — the design sets them all at 24.
    pub const BODY: u32 = 24;
    pub const VALUE: u32 = 32;
    /// The Small overview's single centered value.
    pub const HERO: u32 = 64;
    /// Chart tick labels: smaller than body so they fit their gutters
    /// with a margin off the frame edge.
    pub const TICK: u32 = 20;
}

pub mod space {
    pub const PADDING: f32 = 16.0;
    pub const GAP: f32 = 8.0;
}

const HEADER_LOGO_SIZE: f32 = 24.0;

/// Longest account run the narrowest account-bearing header keeps on one
/// line beside the title — the Medium overview's, whose workers card
/// claims part of the header row. The renderer has no ellipsis, so the
/// header trims the name itself rather than let the row wrap.
const ACCOUNT_MAX_CHARS: usize = 16;

/// The header-left run: logo, widget title, and optionally the bound
/// account's name — all in the design's muted grey.
#[must_use]
pub fn header_left(account: Option<&str>) -> Node {
    let mut children = vec![
        inline_icon(&icons::LOGO, HEADER_LOGO_SIZE, Color::default()),
        text(
            "Braiins Pool",
            style!(size: font::TITLE, weight: FontWeight::SEMIBOLD, color: color::TEXT_MUTED),
        ),
    ];
    if let Some(account) = account {
        let run = if account.chars().count() > ACCOUNT_MAX_CHARS {
            let head: String = account.chars().take(ACCOUNT_MAX_CHARS).collect();
            fmt!("({head}\u{2026})")
        } else {
            fmt!("({account})")
        };
        children.push(text(
            run,
            style!(size: font::TITLE, color: color::TEXT_MUTED),
        ));
    }
    row(props!(gap: 8.0, cross_align: CrossAlign::Center), children)
}

// The typography voices. Every on-screen glyph goes through one of these
// (or a fragment built from them), each pinning `line_height: 1.0` so a
// text line is exactly its font size — the layouts' budgets count on it.

/// Muted body text: titles, labels, subs, units.
#[must_use]
pub fn label(content: &str) -> Node {
    text(
        content,
        style!(size: font::BODY, color: color::TEXT_MUTED, line_height: 1.0),
    )
}

/// The explicit callout for a slot whose source answered empty — data
/// arrived and there is nothing there, as opposed to a loading skeleton.
/// The caller phrases it for its slot ("No payouts yet").
#[must_use]
pub fn absent(callout: &str) -> Node {
    text(
        callout,
        style!(size: font::BODY, color: color::TEXT_ABSENT, line_height: 1.0),
    )
}

/// What a slot says for a source that failed. A slot under a title of its
/// own takes the bare word; an unlabelled one names what it stands for.
pub mod callout {
    pub const UNAVAILABLE: &str = "Unavailable";
    pub const HASHRATE: &str = "Hashrate unavailable";
    pub const HISTORY: &str = "History unavailable";
    pub const LAST_PAYOUT: &str = "Last payout unavailable";
    pub const WORKERS: &str = "Workers unavailable";
}

/// The placeholder for a slot whose source produced no value: `loading`
/// while an answer may still come, `unavailable` once one came back without
/// one. Both nodes are the caller's — a slot's shape is its own business.
#[must_use]
pub fn placeholder<T>(source: &Availability<T>, unavailable: Node, loading: Node) -> Node {
    if source.failed() {
        unavailable
    } else {
        loading
    }
}

/// A value slot's content: the rendered value, or — when its source gave
/// none — the state that explains why. `Loading` may still become a value;
/// `Unavailable` will not.
#[derive(Clone, Copy, Debug)]
pub enum Slot<'a> {
    Value(&'a str),
    Loading,
    Unavailable,
}

impl<'a> Slot<'a> {
    /// The slot for a `value` its caller rendered from `source`.
    #[must_use]
    pub fn new<T>(value: Option<&'a str>, source: &Availability<T>) -> Self {
        match value {
            Some(value) => Self::Value(value),
            None if source.failed() => Self::Unavailable,
            None => Self::Loading,
        }
    }
}

// Loading placeholders come from the SDK's Carbon skeleton builders; the
// widget only fixes the colour and the slots' text sizes.

/// A loading bar for a body-text slot, sized for `chars` glyphs.
#[must_use]
pub fn skeleton(chars: f32) -> Node {
    skeleton::text(chars, font::BODY, color::SKELETON)
}

/// A loading bar for a value slot — Carbon's taller heading bar.
#[must_use]
pub fn skeleton_value(chars: f32, size: u32) -> Node {
    skeleton::heading(chars, size, color::SKELETON)
}

/// A loading block for a chart's plot area.
#[must_use]
pub fn skeleton_block(width: f32, height: f32) -> Node {
    skeleton::placeholder(width, height, color::SKELETON)
}

/// The payout meter's loading state: a track-thick bar in a row of the
/// same height [`meter_row`] gives the real meter, so the card's rhythm
/// holds in either state.
#[must_use]
pub fn skeleton_meter() -> Node {
    row(
        props!(height: METER_ROW_H, cross_align: CrossAlign::Center),
        [skeleton::fill(METER_TRACK_H, color::SKELETON)],
    )
}

/// A callout in a value line's own box: the text stays body-sized, the box
/// keeps the line's height, so a card whose neighbour did load holds its
/// title and footer on the row's shared baselines.
#[must_use]
pub fn absent_value(callout: &str, size: u32) -> Node {
    #[expect(clippy::cast_precision_loss, reason = "a font size is exact in f32")]
    let height = size as f32;
    row(
        props!(height: height, cross_align: CrossAlign::Center),
        [absent(callout)],
    )
}

/// A body line's empty box: the footer of a slot with nothing to say and
/// nothing to wait for.
fn blank_line() -> Node {
    #[expect(clippy::cast_precision_loss, reason = "a font size is exact in f32")]
    let height = font::BODY as f32;
    row(props!(height: height), [])
}

/// A callout centered in a chart's plot area, in place of its loading block.
#[must_use]
pub fn absent_block(width: f32, height: f32, callout: &str) -> Node {
    center(props!(width: width, height: height), [absent(callout)])
}

/// The meter's slot when no payout is underway, spanning the title gap
/// too: the gap above the slot runs wider than the one below, so dead
/// center between the title and the footer — where the thin track reads
/// as floating — lies above the slot itself, out of a slot child's
/// reach. The caller drops its title gap in exchange.
#[must_use]
pub fn absent_meter(callout: &str, gaps: StatGaps) -> Node {
    let span = gaps.label_value + METER_ROW_H;
    #[expect(
        clippy::cast_precision_loss,
        reason = "a small font size is exact in f32"
    )]
    let pad = (span + gaps.value_sub - font::BODY as f32) / 2.0;
    col(
        props!(height: span),
        [row(props!(height: pad), []), absent(callout)],
    )
}

/// A mixed-style body line as ONE paragraph node: cosmic lays every span
/// on a shared baseline, which separate text nodes cannot guarantee —
/// each face's glyph box centers in its own line box. Spans override
/// weight and colour; the family is line-wide (DeckSans, whose digits
/// carry the design's dotted zero).
#[must_use]
pub fn text_run(spans: Vec<Span>) -> Node {
    paragraph(
        style!(size: font::BODY, color: color::TEXT_MUTED, family: FontFamily::DeckSans, line_height: 1.0),
        spans,
    )
}

/// A Bold value span in the series' colour — the hero-stat voice.
#[must_use]
pub fn value_span(value: &str, value_color: Color) -> Span {
    span(value, style!(weight: FontWeight::BOLD, color: value_color))
}

/// A hero stat: grey label run into a Bold value in the series' colour.
/// The label carries its own trailing separator ("5m HR (PH/s): ").
#[must_use]
pub fn stat_pair(pair_label: &str, value: &str, value_color: Color) -> Node {
    text_run(vec![span(pair_label, ()), value_span(value, value_color)])
}

/// A card's box behaviour: padding, whether content centers vertically,
/// and whether the card grows to fill its cell — one preset per design
/// frame that draws cards.
#[derive(Clone, Copy, Debug)]
pub struct CardSpec {
    pub pad_x: f32,
    pub pad_y: f32,
    pub centered: bool,
    pub fill: bool,
}

/// The Medium workers card sits at its natural size.
pub const CARD_M: CardSpec = CardSpec {
    pad_x: 16.0,
    pad_y: 16.0,
    centered: false,
    fill: false,
};
/// Large cards stretch to their row, content at the top.
pub const CARD_L: CardSpec = CardSpec {
    pad_x: 24.0,
    pad_y: 16.0,
    centered: false,
    fill: true,
};
/// Fullscreen tiles stretch, content at the top: with loading skeletons
/// standing in for absent lines, start-justified content never reflows.
pub const CARD_FULL_TILE: CardSpec = CardSpec {
    pad_x: 24.0,
    pad_y: 16.0,
    centered: false,
    fill: true,
};
/// The Fullscreen workers card fills its column top-down.
pub const CARD_FULL_WORKERS: CardSpec = CardSpec {
    pad_x: 16.0,
    pad_y: 16.0,
    centered: false,
    fill: true,
};

/// A bordered stat card. `PropsData` padding is uniform, so the design's
/// wider horizontal padding is emulated with fixed side insets.
#[must_use]
pub fn card(spec: CardSpec, child: Node) -> Node {
    let inset = spec.pad_x - spec.pad_y;
    let body = if inset > 0.0 {
        row(
            props!(),
            [
                col(props!(width: inset), []),
                child,
                col(props!(width: inset), []),
            ],
        )
    } else {
        child
    };
    let justify = if spec.centered {
        Justify::Center
    } else {
        Justify::Start
    };
    let flex = if spec.fill { 1.0 } else { 0.0 };
    col(
        props!(
            background: color::CARD_BG,
            border_radius: 8.0,
            border_width: 1.0,
            border_color: color::CARD_BORDER,
            padding: spec.pad_y,
            justify_content: justify,
            flex: flex,
        ),
        [body],
    )
}

/// Vertical rhythm of a [`stat_block`]: the design tightens the gaps as
/// the frames grow cards around the blocks.
#[derive(Clone, Copy, Debug)]
pub struct StatGaps {
    pub label_value: f32,
    pub value_sub: f32,
}

/// Borderless stat blocks on the Medium overview.
pub const STAT_OPEN: StatGaps = StatGaps {
    label_value: 16.0,
    value_sub: 8.0,
};
/// Carded blocks on the Large overview.
pub const STAT_TIGHT: StatGaps = StatGaps {
    label_value: 8.0,
    value_sub: 4.0,
};
/// Carded blocks on the Fullscreen overview.
pub const STAT_ROOMY: StatGaps = StatGaps {
    label_value: 16.0,
    value_sub: 4.0,
};

/// A label / value / sub stack, optionally led by a legend dot in the
/// colour of the chart line the value belongs to. A slot without a value
/// keeps its place — loading or unavailable, the stack holds still — and
/// `value_chars` sizes the loading bar to the string it stands in for.
#[must_use]
pub fn stat_block(
    dot: Option<Color>,
    block_label: &str,
    value: Slot<'_>,
    sub: Option<&str>,
    gaps: StatGaps,
    value_chars: f32,
) -> Node {
    let title = match dot {
        Some(dot_color) => row(
            props!(gap: 8.0, cross_align: CrossAlign::Center),
            [
                canvas(
                    props!(width: 8.0, height: 8.0),
                    [Draw::circle(4.0, 4.0, 4.0, dot_color)],
                ),
                label(block_label),
            ],
        ),
        None => label(block_label),
    };
    let value_line: Node = match value {
        Slot::Value(value) => text(
            value,
            style!(size: font::VALUE, weight: FontWeight::SEMIBOLD, color: color::TEXT, family: FontFamily::DeckSans, line_height: 1.0),
        ),
        Slot::Loading => skeleton_value(value_chars, font::VALUE),
        Slot::Unavailable => absent_value(callout::UNAVAILABLE, font::VALUE),
    };
    // A sub qualifies a value ("5m Average", "≈ 10.038 USD"), so it goes with
    // one: the slot keeps its height, with nothing left to say or wait for.
    let sub_line = match sub {
        _ if matches!(value, Slot::Unavailable) => blank_line(),
        Some(sub) => label(sub),
        // Subs run "5m Average" to "≈ 10.038 USD".
        None => skeleton(10.0),
    };
    stat_stack(title, value_line, sub_line, gaps)
}

/// The three-slot rhythm every card in a row shares: a title line,
/// a middle slot of the value line's height, and a footer line.
/// One shared stack is what keeps the titles and footers on common
/// baselines, whatever each middle holds — a value, a meter, or a bar.
#[must_use]
pub fn stat_stack(title: Node, middle: Node, footer: Node, gaps: StatGaps) -> Node {
    col(
        props!(gap: gaps.label_value),
        [title, col(props!(gap: gaps.value_sub), [middle, footer])],
    )
}

/// The Small overview's single centered hashrate value.
#[must_use]
pub fn hero_value(value: &str) -> Node {
    text(
        value,
        style!(size: font::HERO, weight: FontWeight::BOLD, color: color::TEXT, family: FontFamily::DeckSans, line_height: 1.0),
    )
}

/// The payout meter's track thickness — heavier than the design's 8 px,
/// which reads as a hairline against the 24 px lines around it.
pub const METER_TRACK_H: f32 = 12.0;

/// The meter's row: as tall as a card's value line, so a payout card keeps
/// the same three-slot rhythm as the stat cards beside it. The row also
/// pins the bar against its node's built-in flex growth.
#[expect(
    clippy::cast_precision_loss,
    reason = "a small font size is exact in f32"
)]
pub const METER_ROW_H: f32 = font::VALUE as f32;

#[must_use]
pub fn meter_row(fraction: f32) -> Node {
    row(
        props!(height: METER_ROW_H, cross_align: CrossAlign::Center),
        [progress_bar(
            "next-payout",
            METER_TRACK_H,
            ProgressMode::Meter(fraction),
            false,
            color::METER_FILL,
            color::METER_TRACK,
            color::CARD_BG,
            None,
        )],
    )
}

/// Icon and colour for a worker state.
#[must_use]
pub fn worker_state_glyph(state: WorkerState) -> (&'static Svg, Color) {
    match state {
        WorkerState::All => (&icons::WORKERS_ALL, color::WORKERS),
        WorkerState::Active => (&icons::WORKERS_OK, color::STATE_OK),
        WorkerState::Low => (&icons::WORKERS_LOW, color::STATE_LOW),
        WorkerState::Offline => (&icons::WORKERS_OFF, color::STATE_OFF),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerState {
    All,
    Active,
    Low,
    Offline,
}

/// A fixed-size inline icon: a canvas of exactly the icon's box.
fn inline_icon(svg: &Svg, size: f32, color: Color) -> Node {
    canvas(
        props!(width: size, height: size),
        [Draw::svg_contain(svg, size, color).with_anti_alias()],
    )
}

/// Row rhythm of the workers panel; the Fullscreen frame spreads the
/// rows out and leads with the all-workers total.
#[derive(Clone, Copy, Debug)]
pub struct WorkersSpec {
    pub include_all: bool,
    pub icon: f32,
    pub count_line_height: f32,
    pub rows_gap: f32,
}

pub const WORKERS_COMPACT: WorkersSpec = WorkersSpec {
    include_all: false,
    icon: 24.0,
    count_line_height: 1.0,
    rows_gap: 8.0,
};
/// The design spaces Fullscreen rows by 12 px pads around 48 px lines;
/// the pads fold into the gap, the line height carries the rest.
pub const WORKERS_ROOMY: WorkersSpec = WorkersSpec {
    include_all: true,
    icon: 32.0,
    count_line_height: 1.5,
    rows_gap: 32.0,
};

/// Workers-by-state panel: a "Workers" title over one icon + count line
/// per state.
#[must_use]
pub fn workers_panel(workers: &Availability<WorkerCounts>, spec: &WorkersSpec) -> Node {
    let line = |state: WorkerState, count: Option<usize>| {
        let count_line = match count {
            Some(count) => text(
                format_number!(count, 0),
                style!(size: font::VALUE, weight: FontWeight::SEMIBOLD, color: color::TEXT, family: FontFamily::DeckSans, line_height: spec.count_line_height),
            ),
            // Counts run to "2 395"; the bar squares off against the icon
            // beside it, which the frames size differently.
            None => skeleton::placeholder(spec.icon * 2.5, spec.icon, color::SKELETON),
        };
        let (svg, glyph_color) = worker_state_glyph(state);
        row(
            props!(gap: 12.0, cross_align: CrossAlign::Center),
            [inline_icon(svg, spec.icon, glyph_color), count_line],
        )
    };
    let rows = |counts: Option<&WorkerCounts>| {
        let mut lines = vec![];
        if spec.include_all {
            // Saturating: `usize` is 32-bit on wasm32, and the parse admits any
            // count up to `u32::MAX`, so four of them need not sum inside one.
            let total = counts.map(|c| {
                c.active
                    .saturating_add(c.low)
                    .saturating_add(c.offline)
                    .saturating_add(c.disabled)
            });
            lines.push(line(WorkerState::All, total));
        }
        lines.push(line(WorkerState::Active, counts.map(|c| c.active)));
        lines.push(line(WorkerState::Low, counts.map(|c| c.low)));
        lines.push(line(WorkerState::Offline, counts.map(|c| c.offline)));
        col(props!(gap: spec.rows_gap), lines)
    };
    // One callout for the panel, not the same word down every state's row.
    let body = placeholder(
        workers,
        absent(callout::UNAVAILABLE),
        rows(workers.as_option()),
    );

    col(props!(gap: 10.0), vec![label("Workers"), body])
}

/// Where the unbound state points the operator: the Deck web app on the
/// current network. Empty strings drop their line, down to the bare
/// instruction when the network is unknown.
#[derive(Clone, Debug, Default)]
pub struct BindHint {
    pub ssid: String,
    pub url: String,
}

const BIND_LINE_GAP: f32 = 12.0;

/// Leading inside the hint's wrapped prose — the widget's only wrapping
/// copy, so unlike the single-line voices it needs real leading. Declared
/// rather than left to the default so the QR can match the block's height.
const BIND_LINE_HEIGHT: f32 = 1.3;

/// The instruction's wrapped line count at every cap `unbound_body` sets.
const BIND_INSTRUCTION_LINES: f32 = 2.0;

/// The hint text block's height, so a QR beside it squares off against it:
/// the instruction's wrapped lines plus one per extra line, each at the
/// hint's leading, with a gap between the blocks.
#[expect(
    clippy::cast_precision_loss,
    reason = "a small font size and line count are exact in f32"
)]
fn bind_text_height(extra_lines: usize) -> f32 {
    let line = font::BODY as f32 * BIND_LINE_HEIGHT;
    (BIND_INSTRUCTION_LINES + extra_lines as f32) * line + BIND_LINE_GAP * extra_lines as f32
}

/// Unbound-state body: centered bind instructions, led by a QR code to the
/// Deck web app where the frame fits one.
#[must_use]
pub fn unbound_body(bucket: SizeBucket, hint: &BindHint) -> Node {
    // The wide frames run the QR beside the text; the narrow ones stack it.
    let beside = matches!(bucket, SizeBucket::Medium | SizeBucket::Full);
    let text_align = if beside {
        TextAlign::Left
    } else {
        TextAlign::Center
    };
    // The smallest frame fits the instruction and the address only; the
    // wrapped full sentence plus a network line runs past its 220 px.
    let compact = bucket == SizeBucket::Small;
    let instruction = if compact {
        "Bind an account in the Deck web app"
    } else {
        "Bind a Braiins Pool account in the Deck web app to see your stats."
    };
    let mut lines = vec![text(
        instruction,
        style!(size: font::BODY, color: color::TEXT, align: text_align, line_height: BIND_LINE_HEIGHT),
    )];
    if !compact && !hint.ssid.is_empty() {
        lines.push(text(
            fmt!("On the network \u{201c}{}\u{201d}", hint.ssid),
            style!(size: font::BODY, color: color::TEXT_MUTED, align: text_align, line_height: BIND_LINE_HEIGHT),
        ));
    }
    if !hint.url.is_empty() {
        lines.push(text(
            hint.url.as_str(),
            style!(size: font::BODY, color: color::WORKERS, align: text_align, line_height: BIND_LINE_HEIGHT),
        ));
    }
    // Beside the text the QR takes the text block's exact height, so their
    // top and bottom edges line up; stacked above it, the frame's own size.
    let qr_size = if beside {
        Some(bind_text_height(lines.len() - 1))
    } else {
        match bucket {
            SizeBucket::Large => Some(170.0),
            SizeBucket::Small | SizeBucket::Medium | SizeBucket::Full => None,
        }
    };
    let qr_code = qr_size.filter(|_| !hint.url.is_empty()).map(|size| {
        canvas(
            props!(width: size, height: size),
            [Draw::qr(0.0, 0.0, size, &hint.url, QrStyle::default())],
        )
    });
    let mut cells: Vec<Node> = qr_code.into_iter().collect();
    // Cap the run so long instruction lines wrap instead of overflowing
    // the centered box, and so the wrap lands on balanced lines rather
    // than a full line over a stub: the stacked frames cap near half the
    // sentence's width, Medium near the width left beside its QR.
    let max_w = match bucket {
        SizeBucket::Small => 274.0,
        SizeBucket::Medium => 572.0,
        SizeBucket::Large => 440.0,
        SizeBucket::Full => 700.0,
    };
    let content = if beside {
        cells.push(col(props!(gap: 12.0), lines));
        row(
            props!(gap: 32.0, cross_align: CrossAlign::Center, max_width: max_w),
            cells,
        )
    } else {
        cells.append(&mut lines);
        col(
            props!(gap: 16.0, cross_align: CrossAlign::Center, max_width: max_w),
            cells,
        )
    };
    center(props!(flex: 1.0), [content])
}

/// Denied-state body: the API recognizes the key but refuses its reads
/// (HTTP 401/403), so waiting will not help — the key must be reissued
/// with monitoring access.
#[must_use]
pub fn denied_body(bucket: SizeBucket) -> Node {
    // One text node per line: free wrapping breaks wherever the width
    // runs out; these breaks keep the lines balanced.
    //
    // The short frames fit a nudge toward the key's settings and nothing more
    // — 140 px of body against 190 for the full stack — and Medium runs
    // that nudge on one line, being the wide one of the two.
    let detail: &[&str] = match bucket {
        SizeBucket::Small => &["Check the API key's", "permissions"],
        SizeBucket::Medium => &["Check the API key's permissions"],
        SizeBucket::Large => &[
            "The account's API key cannot read pool stats.",
            "Reissue it in the pool's settings",
            "with monitoring access.",
        ],
        SizeBucket::Full => &[
            "The account's API key cannot read pool stats.",
            "Reissue it in the pool's settings with monitoring access.",
        ],
    };
    let (icon, gap) = if bucket == SizeBucket::Small {
        (32.0, space::GAP)
    } else {
        (48.0, 12.0)
    };
    let lines: Vec<Node> = detail
        .iter()
        .map(|line| {
            text(
                *line,
                style!(size: font::BODY, color: color::TEXT_MUTED, line_height: BIND_LINE_HEIGHT),
            )
        })
        .collect();
    center(
        props!(flex: 1.0),
        [col(
            props!(gap: gap, cross_align: CrossAlign::Center),
            [
                canvas(
                    props!(width: icon, height: icon),
                    [Draw::svg_builtin(
                        0.0,
                        0.0,
                        icon,
                        icon,
                        ICON_ERROR,
                        color::ERROR,
                    )],
                ),
                text(
                    "Access denied",
                    style!(size: font::TITLE, weight: FontWeight::SEMIBOLD, color: color::TEXT, line_height: 1.0),
                ),
                col(props!(cross_align: CrossAlign::Center), lines),
            ],
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_shows_account_when_bound() {
        // Node has no public inspection API; presence of both texts is
        // covered visually by the storybook and captures. This test pins
        // that assembly does not panic on either shape.
        bmc_wasm_sdk::assets::init_test_registrars();
        let _ = header_left(Some("user.braiins"));
        let _ = header_left(None);
    }

    #[test]
    fn workers_panel_assembles_in_both_rhythms() {
        bmc_wasm_sdk::assets::init_test_registrars();
        let counts = Availability::Available(WorkerCounts {
            active: 1_628,
            low: 758,
            offline: 102,
            disabled: 7,
        });
        let _ = workers_panel(&counts, &WORKERS_COMPACT);
        let _ = workers_panel(&counts, &WORKERS_ROOMY);
        let _ = workers_panel(&Availability::default(), &WORKERS_ROOMY);
    }
}

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

#[expect(
    clippy::wildcard_imports,
    reason = "render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use units::availability::Availability;
use units::format::{Rendered, fixed};
use units::units::{DegreeCelsius, Quantity};

use crate::layout::{Layout, choose, truncate_label};
use crate::paging;
use crate::summary::{FleetSummary, GroupSummary};
use crate::view::details_click_id;

const OK_ICON: Svg = include_svg!("assets/ok.svg");
const NOT_OK_ICON: Svg = include_svg!("assets/not-ok.svg");
const CHEVRON_LEFT: Svg = include_svg!("assets/chevron-left.svg");
const CHEVRON_RIGHT: Svg = include_svg!("assets/chevron-right.svg");
const ICON_PX: f32 = 22.0;

const LABEL_COLOR: Color = GRAY_60;
const VALUE_COLOR: Color = WHITE;
const OK_COLOR: Color = GREEN_50;
const NOT_OK_COLOR: Color = RED_60;

const LABEL_FONT: u32 = 18;
const VALUE_FONT: u32 = 28;
const ROW_FONT: u32 = 26;
// The hero hashrate is smaller in the narrow Large band so a large fleet total
// still fits beside the counts and title without wrapping.
const HERO_FONT_FULL: u32 = 48;
const HERO_FONT_LARGE: u32 = 40;
// The title is large in the wide Full band; the narrow Large band can't fit a
// 64px title beside the hero hashrate and counts, so it uses a smaller one.
const TITLE_FONT_FULL: u32 = 64;
const TITLE_FONT_LARGE: u32 = 32;
// The detail view's title is a model name, not the operator's short fleet
// name, and shares its row with the Back button; a 64px title cannot fit a
// typical model name in the Full band, so the detail title steps down and
// long names truncate to the band's character budget.
const DETAIL_TITLE_FONT_FULL: u32 = 48;
const DETAIL_TITLE_CHARS_FULL: usize = 18;
const DETAIL_TITLE_CHARS_LARGE: usize = 12;

// The summary-only layout's fixed fonts (small viewports such as the BMM101).
const SUMMARY_TITLE_FONT: u32 = 32;
const SUMMARY_LABEL_FONT: u32 = 20;
const SUMMARY_VALUE_FONT: u32 = 28;

// A dark, subtle hairline matching the weather widget's `BORDER` (GRAY_70).
const SEPARATOR_COLOR: Color = GRAY_70;
const SEPARATOR_PX: f32 = 1.0;

// Uniform gap between every breakdown column.
const ROW_GAP: f32 = 12.0;

// Fixed breakdown column widths, shared by the header and data rows so labels
// sit over their values. Each is its column's widest expected content plus a
// roughly constant slack, so left-aligned values land at evenly spaced
// positions for typical data (a much shorter value leaves more trailing space).
// The widths also budget for the right-aligned pager cluster on the header row
// (48 + 8 + ~28 + 8 + 48 ≈ 140 px plus its 12 px column gap); each data row
// right-aligns a labeled Details button (~105 px) into the same remainder.
// The model column is the widest the header budget allows so full model names
// such as "Braiins Mini Miner BMM 101" show untruncated in the Full band; the
// Large model column relies on code-side truncation.
const COL_HASHRATE: f32 = 140.0;
const COL_MODEL_FULL: f32 = 375.0;
const COL_MODEL_LARGE: f32 = 180.0;
const COL_POWER: f32 = 80.0;
const COL_EFF: f32 = 130.0;
const COL_TEMP: f32 = 150.0;
const COL_COUNTS: f32 = 130.0;
// The Large band fits only three columns into 638px, so the model and status
// columns share a tight budget. The status content is left-packed with a narrow
// tail, so the model borrows that slack here to hold longer model names without
// wrapping; the wide Full band keeps the roomier `COL_COUNTS`. The header
// row packs 140 + 180 + 70 columns, four gaps (the spacer doubles the
// trailing one), and the ~140 px pager cluster into ~580 of the 590 px
// content box.
const COL_COUNTS_LARGE: f32 = 70.0;

// The detail view's Name column in the Large band shares the fleet table's
// header budget.
const COL_NAME_LARGE: f32 = 180.0;
const NAME_CHARS_FULL: usize = 26;
const NAME_CHARS_LARGE: usize = 12;

// Character budgets for the model column; longer names are cut with an
// ellipsis so rows never wrap (the engine cannot ellipsize).
const MODEL_CHARS_FULL: usize = 26;
const MODEL_CHARS_LARGE: usize = 12;

fn value_string(rendered: Rendered) -> String {
    match rendered.unit {
        Some(unit) => fmt!("{} {}", rendered.value, unit),
        None => rendered.value,
    }
}

fn hashrate_str<Q: Quantity>(value: Availability<Q>) -> String {
    value_string(fixed(value, 2))
}

fn whole_str<Q: Quantity>(value: Availability<Q>) -> String {
    value_string(fixed(value, 0))
}

fn tenth_str<Q: Quantity>(value: Availability<Q>) -> String {
    value_string(fixed(value, 1))
}

// The bare integer magnitude (no unit); "N/A" when unavailable.
fn whole_value<Q: Quantity>(value: Availability<Q>) -> String {
    fixed(value, 0).value
}

// `min/avg/max °C` over the group's reporting devices; "N/A" when none report.
// The three collapse to one value for a single-device group.
fn temp_range_str(group: &GroupSummary) -> String {
    if group.max_temperature.as_option().is_none() {
        return fixed(group.max_temperature, 0).value;
    }
    fmt!(
        "{}/{}/{} {}",
        whole_value(group.min_temperature),
        whole_value(group.avg_temperature),
        whole_value(group.max_temperature),
        DegreeCelsius::UNIT,
    )
}

// A label above its value, the overview cluster's building block; `value_font`
// and `value_weight` let the headline hashrate render larger and bolder than
// the secondary metrics.
fn metric(label: &str, value: String, value_font: u32, value_weight: FontWeight) -> Node {
    col(
        props!(gap: 4.0),
        [
            text(label, style!(size: LABEL_FONT, color: LABEL_COLOR)),
            text(
                value,
                style!(size: value_font, weight: value_weight, color: VALUE_COLOR),
            ),
        ],
    )
}

// The status cluster as an overview column.
fn status(total: &GroupSummary) -> Node {
    col(
        props!(gap: 4.0),
        [
            text("Status", style!(size: LABEL_FONT, color: LABEL_COLOR)),
            counts(total.ok_count, not_ok_count(total), VALUE_FONT),
        ],
    )
}

// A full-width hairline between the summary and the per-model breakdown.
fn separator() -> Node {
    col(
        props!(background: SEPARATOR_COLOR, height: SEPARATOR_PX),
        Vec::<Node>::new(),
    )
}

// Not-okay devices: every known device in the group that is not mining
// (reachable-but-idle, errored, or unreachable).
fn not_ok_count(group: &GroupSummary) -> usize {
    group.total_count - group.ok_count
}

// A status icon beside its count, the building block of the status cluster.
fn status_pair(icon: &Svg, color: Color, count: usize, font: u32) -> Node {
    row(
        props!(gap: 6.0, cross_align: CrossAlign::Center),
        [
            canvas(
                props!(width: ICON_PX, height: ICON_PX),
                vec![Draw::svg(0.0, 0.0, ICON_PX, ICON_PX, icon, color)],
            ),
            text(
                format_number!(ok_count_f64(count), 0),
                style!(size: font, color: VALUE_COLOR),
            ),
        ],
    )
}

// The okay/not-okay status counts: a green check with the mining count and a
// red `!` with the not-okay count. Each pair appears only when its count is
// above zero; every group has at least one device, so at least one is shown.
fn counts(ok: usize, not_ok: usize, font: u32) -> Node {
    let mut pairs: Vec<Node> = Vec::new();
    if ok > 0 {
        pairs.push(status_pair(&OK_ICON, OK_COLOR, ok, font));
    }
    if not_ok > 0 {
        pairs.push(status_pair(&NOT_OK_ICON, NOT_OK_COLOR, not_ok, font));
    }
    row(props!(gap: 16.0, cross_align: CrossAlign::Center), pairs)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "fleet device counts stay within f64's exact integer range"
)]
fn ok_count_f64(count: usize) -> f64 {
    count as f64
}

// The headline hashrate and ok/online counts always lead; Power and Efficiency
// follow only in the wide Full band — four summary columns plus the hero value
// do not fit the 638px Large band without wrapping.
fn overview(total: &GroupSummary, variant: SizeVariant, title: &str, back: bool) -> Node {
    let full = matches!(variant, SizeVariant::Full);
    let hero_font = if full {
        HERO_FONT_FULL
    } else {
        HERO_FONT_LARGE
    };
    let mut cells = vec![metric(
        "Total Hashrate",
        hashrate_str(total.hashrate),
        hero_font,
        FontWeight::BOLD,
    )];
    // The Large band's 590px content box cannot hold the hero, the counts,
    // the title and the Back button on one row; the detail view drops the
    // counts there (every device row below carries its own status icon).
    if full || !back {
        cells.push(status(total));
    }
    if full {
        cells.push(metric(
            "Power",
            whole_str(total.power),
            VALUE_FONT,
            FontWeight::REGULAR,
        ));
        cells.push(metric(
            "Efficiency",
            tenth_str(total.efficiency),
            VALUE_FONT,
            FontWeight::REGULAR,
        ));
    }
    cells.push(spacer(1.0));
    let title_font = match (full, back) {
        (true, false) => TITLE_FONT_FULL,
        (true, true) => DETAIL_TITLE_FONT_FULL,
        (false, _) => TITLE_FONT_LARGE,
    };
    cells.push(text(
        title,
        style!(size: title_font, weight: FontWeight::SEMIBOLD, color: VALUE_COLOR),
    ));
    if back {
        cells.push(button!(
            "back",
            "Back",
            style: Ghost,
            size: Normal,
            icon: ensure_registered(&CHEVRON_LEFT)
        ));
    }
    row(props!(gap: 32.0, cross_align: CrossAlign::Start), cells)
}

// A fixed-width breakdown column. The width packs columns into a compact table
// (empty space falls to the right margin) instead of flex-stretching each cell
// across the viewport; the box width is what lets `align` right-justify values.
fn text_cell(width: f32, value: String, color: Color, align: TextAlign) -> Node {
    col(
        props!(width: width),
        [text(
            value,
            style!(size: ROW_FONT, color: color, align: align),
        )],
    )
}

fn counts_cell(width: f32, group: &GroupSummary) -> Node {
    col(
        props!(width: width),
        [counts(group.ok_count, not_ok_count(group), ROW_FONT)],
    )
}

// The breakdown column-label row, using the same widths as the data rows. The
// labels render at LABEL_FONT, matching the overview cluster labels.
fn header_cell(width: f32, label: &str) -> Node {
    col(
        props!(width: width),
        [text(label, style!(size: LABEL_FONT, color: LABEL_COLOR))],
    )
}

// The per-view column widths the header and data rows share.
struct TableColumns {
    label_w: f32,
    label_chars: usize,
    counts_w: f32,
}

fn fleet_columns(variant: SizeVariant) -> TableColumns {
    if matches!(variant, SizeVariant::Full) {
        TableColumns {
            label_w: COL_MODEL_FULL,
            label_chars: MODEL_CHARS_FULL,
            counts_w: COL_COUNTS,
        }
    } else {
        TableColumns {
            label_w: COL_MODEL_LARGE,
            label_chars: MODEL_CHARS_LARGE,
            counts_w: COL_COUNTS_LARGE,
        }
    }
}

fn detail_columns(variant: SizeVariant) -> TableColumns {
    if matches!(variant, SizeVariant::Full) {
        TableColumns {
            label_w: COL_MODEL_FULL,
            label_chars: NAME_CHARS_FULL,
            counts_w: COL_COUNTS,
        }
    } else {
        TableColumns {
            label_w: COL_NAME_LARGE,
            label_chars: NAME_CHARS_LARGE,
            counts_w: COL_COUNTS_LARGE,
        }
    }
}

struct Pager {
    page: usize,
    count: usize,
}

// Chevron pager flanking the page indicator, right-aligned on the header
// row. The buttons disable (not vanish) at the bounds so the cluster never
// reflows under a mid-pagination finger.
fn pager_cluster(pager: &Pager) -> Node {
    row(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            button!(
                "page_prev",
                "",
                style: Ghost,
                size: Normal,
                icon: ensure_registered(&CHEVRON_LEFT),
                disabled: pager.page == 0
            ),
            text(
                fmt!("{}/{}", pager.page + 1, pager.count),
                style!(size: LABEL_FONT, color: LABEL_COLOR),
            ),
            button!(
                "page_next",
                "",
                style: Ghost,
                size: Normal,
                icon: ensure_registered(&CHEVRON_RIGHT),
                disabled: pager.page + 1 >= pager.count
            ),
        ],
    )
}

fn header_row(variant: SizeVariant, cols: &TableColumns, label: &str, pager: &Pager) -> Node {
    let full = matches!(variant, SizeVariant::Full);
    let mut cells = vec![
        header_cell(COL_HASHRATE, "Hashrate"),
        header_cell(cols.label_w, label),
    ];
    if full {
        cells.push(header_cell(COL_POWER, "Power"));
        cells.push(header_cell(COL_EFF, "Efficiency"));
        cells.push(header_cell(COL_TEMP, "Temp"));
    }
    cells.push(header_cell(cols.counts_w, "Status"));
    if pager.count > 1 {
        cells.push(spacer(1.0));
        cells.push(pager_cluster(pager));
    }
    row(props!(gap: ROW_GAP, cross_align: CrossAlign::Center), cells)
}

// Full shows the whole table; the narrower Large band keeps only the headline
// hashrate, the model, and the ok/online counts — the long model names cannot
// share a 638px row with the numeric columns without wrapping.
fn breakdown_row(group: &GroupSummary, variant: SizeVariant, cols: &TableColumns) -> Node {
    let full = matches!(variant, SizeVariant::Full);
    let mut cells = vec![
        text_cell(
            COL_HASHRATE,
            hashrate_str(group.hashrate),
            VALUE_COLOR,
            TextAlign::Left,
        ),
        text_cell(
            cols.label_w,
            truncate_label(&group.label, cols.label_chars),
            VALUE_COLOR,
            TextAlign::Left,
        ),
    ];
    if full {
        cells.push(text_cell(
            COL_POWER,
            whole_str(group.power),
            VALUE_COLOR,
            TextAlign::Left,
        ));
        cells.push(text_cell(
            COL_EFF,
            tenth_str(group.efficiency),
            VALUE_COLOR,
            TextAlign::Left,
        ));
        cells.push(text_cell(
            COL_TEMP,
            temp_range_str(group),
            VALUE_COLOR,
            TextAlign::Left,
        ));
    }
    cells.push(counts_cell(cols.counts_w, group));
    cells.push(spacer(1.0));
    cells.push(button!(
        details_click_id(group.family, &group.label),
        "Details",
        style: Tertiary,
        size: Large
    ));
    row(props!(gap: ROW_GAP, cross_align: CrossAlign::Center), cells)
}

// A value text node in the summary layout.
fn summary_value(value: String) -> Node {
    text(value, style!(size: SUMMARY_VALUE_FONT, color: VALUE_COLOR))
}

// A centered title header. Flanking spacers center it horizontally — the engine
// has no main-axis justify.
fn summary_title(title: &str) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            spacer(1.0),
            text(
                title,
                style!(size: SUMMARY_TITLE_FONT, weight: FontWeight::SEMIBOLD, color: VALUE_COLOR),
            ),
            spacer(1.0),
        ],
    )
}

// A label-left / value-right metric row; the spacer pushes the value to the
// right edge.
fn metric_row(label: &str, value: Node) -> Node {
    row(
        props!(cross_align: CrossAlign::Center),
        [
            text(label, style!(size: SUMMARY_LABEL_FONT, color: LABEL_COLOR)),
            spacer(1.0),
            value,
        ],
    )
}

// The summary-only screen for viewports too small for the breakdown table: a
// centered title above the fleet totals, rows spread to fill the height.
fn summary_view(total: &GroupSummary, title: &str) -> Node {
    col(
        props!(background: BLACK, padding: 24.0),
        [
            summary_title(title),
            spacer(1.0),
            metric_row("Hashrate", summary_value(hashrate_str(total.hashrate))),
            spacer(1.0),
            metric_row(
                "Status",
                counts(total.ok_count, not_ok_count(total), SUMMARY_VALUE_FONT),
            ),
            spacer(1.0),
            metric_row("Power", summary_value(whole_str(total.power))),
            spacer(1.0),
            metric_row("Efficiency", summary_value(tenth_str(total.efficiency))),
            spacer(1.0),
        ],
    )
}

// A single ok/not-ok icon: a one-device row has no counts to show.
fn status_icon_cell(width: f32, ok: bool) -> Node {
    let (icon, color) = if ok {
        (&OK_ICON, OK_COLOR)
    } else {
        (&NOT_OK_ICON, NOT_OK_COLOR)
    };
    col(
        props!(width: width),
        [canvas(
            props!(width: ICON_PX, height: ICON_PX),
            vec![Draw::svg(0.0, 0.0, ICON_PX, ICON_PX, icon, color)],
        )],
    )
}

// One device of the drilled-into model: the model columns with the device
// name in the model slot, a single temperature value, and a single status
// icon.
fn device_row(device: &GroupSummary, variant: SizeVariant, cols: &TableColumns) -> Node {
    let full = matches!(variant, SizeVariant::Full);
    let mut cells = vec![
        text_cell(
            COL_HASHRATE,
            hashrate_str(device.hashrate),
            VALUE_COLOR,
            TextAlign::Left,
        ),
        text_cell(
            cols.label_w,
            truncate_label(&device.label, cols.label_chars),
            VALUE_COLOR,
            TextAlign::Left,
        ),
    ];
    if full {
        cells.push(text_cell(
            COL_POWER,
            whole_str(device.power),
            VALUE_COLOR,
            TextAlign::Left,
        ));
        cells.push(text_cell(
            COL_EFF,
            tenth_str(device.efficiency),
            VALUE_COLOR,
            TextAlign::Left,
        ));
        cells.push(text_cell(
            COL_TEMP,
            whole_str(device.avg_temperature),
            VALUE_COLOR,
            TextAlign::Left,
        ));
    }
    cells.push(status_icon_cell(cols.counts_w, device.ok_count > 0));
    row(props!(gap: ROW_GAP, cross_align: CrossAlign::Center), cells)
}

/// The drilled-into model the renderer shows instead of the fleet table.
pub struct DetailData<'a> {
    pub group: &'a GroupSummary,
    pub rows: &'a [GroupSummary],
    pub page: usize,
}

/// The built frame plus the page count of whatever table it shows, so the
/// click handler clamps against the exact view that was rendered.
pub struct Frame {
    pub root: Node,
    pub page_count: usize,
}

// The model's own totals in the overview (title = model name, with Back),
// then one page of device rows.
fn detail_view(detail: &DetailData<'_>, height: u32, variant: SizeVariant) -> Frame {
    let cols = detail_columns(variant);
    let per_page = paging::rows_per_page_detail(height, variant);
    let count = paging::page_count(detail.rows.len(), per_page);
    let page = paging::effective_page(detail.page, count);
    let pager = Pager { page, count };
    let bounds = paging::page_bounds(detail.rows.len(), per_page, page);

    let title_chars = if matches!(variant, SizeVariant::Full) {
        DETAIL_TITLE_CHARS_FULL
    } else {
        DETAIL_TITLE_CHARS_LARGE
    };
    let title = truncate_label(&detail.group.label, title_chars);
    let mut children: Vec<Node> = vec![
        overview(detail.group, variant, &title, true),
        separator(),
        header_row(variant, &cols, "Name", &pager),
    ];
    for device in detail
        .rows
        .get(bounds)
        .expect("BUG: effective page bounds are in range")
    {
        children.push(device_row(device, variant, &cols));
    }

    Frame {
        root: col(props!(background: BLACK, padding: 24.0, gap: 5.8), children),
        page_count: count,
    }
}

// The overview row plus one page of the per-model breakdown table.
fn table_view(
    summary: &FleetSummary,
    fleet_page: usize,
    height: u32,
    variant: SizeVariant,
    title: &str,
) -> Frame {
    let cols = fleet_columns(variant);
    let per_page = paging::rows_per_page_fleet(height, variant);
    let count = paging::page_count(summary.groups.len(), per_page);
    let page = paging::effective_page(fleet_page, count);
    let pager = Pager { page, count };
    let bounds = paging::page_bounds(summary.groups.len(), per_page, page);

    let mut children: Vec<Node> =
        vec![overview(&summary.total, variant, title, false), separator()];
    children.push(header_row(variant, &cols, "Model", &pager));
    for group in summary
        .groups
        .get(bounds)
        .expect("BUG: effective page bounds are in range")
    {
        children.push(breakdown_row(group, variant, &cols));
    }

    Frame {
        root: col(props!(background: BLACK, padding: 24.0, gap: 5.8), children),
        page_count: count,
    }
}

#[must_use]
pub fn view(
    summary: &FleetSummary,
    detail: Option<DetailData<'_>>,
    fleet_page: usize,
    width: u32,
    height: u32,
    variant: SizeVariant,
    title: &str,
) -> Frame {
    // No visible groups means nothing to show yet: no devices, none polled
    // successfully, or all filtered out by model lists / disabled families.
    if summary.groups.is_empty() {
        return Frame {
            root: col(
                props!(background: BLACK),
                [center(
                    props!(flex: 1.0),
                    [text(
                        "Searching for miners\u{2026}",
                        style!(size: 28, color: WHITE),
                    )],
                )],
            ),
            page_count: 1,
        };
    }

    match choose(width, height) {
        Layout::Summary => Frame {
            root: summary_view(&summary.total, title),
            page_count: 1,
        },
        Layout::Table => match detail {
            Some(detail) => detail_view(&detail, height, variant),
            None => table_view(summary, fleet_page, height, variant, title),
        },
    }
}

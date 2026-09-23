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

//! What every layout is built from: the palette, the type scale,
//! the countdown row and the info tiles.

#[expect(
    clippy::wildcard_imports,
    reason = "screen fragments use the SDK's tree builders, macros, and tokens throughout"
)]
use bmc_wasm_sdk::*;

use crate::manifest_params::NumbersFontStyle;
use crate::model::Countdown;

pub(super) const NOT_AVAILABLE: &str = "-";

pub(super) const TILE_BG: Color = Color::from_hex(0x0F_0F_0F);
const TITLE_COLOR: Color = GRAY_60;
const NUMERAL_COLOR: Color = WHITE;
/// One em, so the units sit just under the digits as the Deck design draws them,
/// not below the empty band the default 1.4 line box leaves under the digits.
const NUMERAL_LINE_HEIGHT: f32 = 1.0;
const LABEL_COLOR: Color = GRAY_60;
const TILE_VALUE_COLOR: Color = WHITE;
const TILE_SUB_COLOR: Color = GRAY_60;

/// Font sizes for the three text rows of a bottom tile.
#[derive(Clone, Copy)]
pub(super) struct TileSizes {
    pub label: u32,
    pub value: u32,
    pub sub: u32,
}

/// Which layout a size renders: one compact countdown tile (Small/Medium, the round face),
/// or the countdown over the predicted-date and blocks-remaining tiles (Large/Full).
#[derive(Clone, Copy)]
pub(super) enum Layout {
    Compact,
    Tiled(TileSizes),
}

pub(super) struct SizeParams {
    pub layout: Layout,
    pub title: u32,
    pub numeral: u32,
    pub label: u32,
    pub padding: f32,
    pub gap: f32,
}

impl SizeParams {
    /// Downscale every font and spacing by `fit` (`WidgetSize::fit`):
    /// the viewport's ratio to its variant's canonical box, 1.0 at or above it.
    pub(super) fn scaled(&self, fit: f32) -> Self {
        let layout = match self.layout {
            Layout::Compact => Layout::Compact,
            Layout::Tiled(t) => Layout::Tiled(TileSizes {
                label: scale_font(t.label, fit),
                value: scale_font(t.value, fit),
                sub: scale_font(t.sub, fit),
            }),
        };
        Self {
            layout,
            title: scale_font(self.title, fit),
            numeral: scale_font(self.numeral, fit),
            label: scale_font(self.label, fit),
            padding: self.padding * fit,
            gap: self.gap * fit,
        }
    }
}

pub(super) fn numeral_weight(style: NumbersFontStyle) -> FontWeight {
    match style {
        NumbersFontStyle::Regular => FontWeight::REGULAR,
        NumbersFontStyle::SemiBold => FontWeight::SEMIBOLD,
        NumbersFontStyle::Bold => FontWeight::BOLD,
    }
}

const FOUR_DIGIT_DAYS: i64 = 1_000;

/// The countdown place-values as display strings, `-` when there is no prediction.
pub(super) struct Numerals {
    pub days: String,
    pub hours: String,
    /// `None` where the layout has no room left for them.
    pub minutes: Option<String>,
}

impl Numerals {
    pub(super) fn of(countdown: Option<Countdown>) -> Self {
        match countdown {
            Some(countdown) => Self {
                days: int_string(countdown.days),
                hours: pad2(countdown.hours),
                minutes: Some(pad2(countdown.minutes)),
            },
            None => Self {
                days: NOT_AVAILABLE.to_owned(),
                hours: NOT_AVAILABLE.to_owned(),
                minutes: Some(NOT_AVAILABLE.to_owned()),
            },
        }
    }

    /// Four-digit days drop the minutes:
    /// a narrow layout fits `DDDD : HH` where it has no room for `DDDD : HH : MM`.
    pub(super) fn narrow(countdown: Option<Countdown>) -> Self {
        let mut numerals = Self::of(countdown);
        if countdown.is_some_and(|countdown| countdown.days >= FOUR_DIGIT_DAYS) {
            numerals.minutes = None;
        }
        numerals
    }
}

/// Two-digit zero-padded, via the SDK's no-fmt writer:
/// widget code must avoid `core::fmt` (the `no-fmt-in-wasm` check).
fn pad2(n: i64) -> String {
    let mut s = String::new();
    format::push_pad2(&mut s, n);
    s
}

fn int_string(n: i64) -> String {
    let mut s = String::new();
    format::push_int(&mut s, n);
    s
}

pub(super) fn title(size: &SizeParams) -> Node {
    text(
        "Halving Countdown",
        style!(size: size.title, weight: FontWeight::REGULAR, color: TITLE_COLOR),
    )
}

fn numeral(value: impl Into<String>, size: &SizeParams, weight: FontWeight) -> Node {
    text(
        value,
        style!(
            size: size.numeral,
            weight: weight,
            color: NUMERAL_COLOR,
            family: FontFamily::DeckSans,
            line_height: NUMERAL_LINE_HEIGHT,
        ),
    )
}

fn label(value: &str, size: &SizeParams) -> Node {
    text(
        value,
        style!(size: size.label, weight: FontWeight::REGULAR, color: LABEL_COLOR),
    )
}

/// One `DD` / `HH` / `MM` column: big numeral over its label.
fn column(value: String, name: &str, size: &SizeParams, weight: FontWeight) -> Node {
    col(
        props!(cross_align: CrossAlign::Center),
        [numeral(value, size, weight), label(name, size)],
    )
}

fn colon(size: &SizeParams, weight: FontWeight) -> Node {
    numeral(":", size, weight)
}

/// The `DD : HH : MM` row, minutes only when the numerals carry them:
/// numeral-over-label columns with colons between, top-aligned
/// so the colons sit with the numerals and each label stays centered under its own number.
pub(super) fn countdown_columns(
    numerals: Numerals,
    size: &SizeParams,
    weight: FontWeight,
    gap: f32,
) -> Node {
    let mut columns = vec![
        column(numerals.days, "Days", size, weight),
        colon(size, weight),
        column(numerals.hours, "Hours", size, weight),
    ];
    if let Some(minutes) = numerals.minutes {
        columns.push(colon(size, weight));
        columns.push(column(minutes, "Min.", size, weight));
    }
    row(props!(gap: gap, cross_align: CrossAlign::Start), columns)
}

/// A bottom tile: caption, big value, sub-line.
/// Without data the value reads `-` and the sub-line is empty.
pub(super) fn info_tile(
    caption: &str,
    value: Option<String>,
    sub: Option<String>,
    tiles: TileSizes,
) -> Node {
    let value = value.unwrap_or_else(|| NOT_AVAILABLE.to_owned());
    let sub = sub.unwrap_or_default();
    col(
        props!(background: TILE_BG, flex: 1.0),
        [center(
            props!(flex: 1.0),
            [col(
                props!(gap: 12.0, cross_align: CrossAlign::Center),
                [
                    text(
                        caption,
                        style!(size: tiles.label, weight: FontWeight::REGULAR, color: TILE_SUB_COLOR),
                    ),
                    text(
                        value,
                        style!(
                            size: tiles.value,
                            weight: FontWeight::BOLD,
                            color: TILE_VALUE_COLOR,
                            family: FontFamily::DeckSans,
                        ),
                    ),
                    text(
                        sub,
                        style!(size: tiles.sub, weight: FontWeight::REGULAR, color: TILE_SUB_COLOR),
                    ),
                ],
            )],
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn countdown(days: i64) -> Option<Countdown> {
        Some(Countdown {
            days,
            hours: 23,
            minutes: 59,
        })
    }

    #[test]
    fn narrow_numerals_drop_the_minutes_from_four_digit_days_on() {
        assert_eq!(
            Numerals::narrow(countdown(999)).minutes.as_deref(),
            Some("59")
        );
        assert_eq!(Numerals::narrow(countdown(1_000)).minutes, None);
    }

    #[test]
    fn narrow_numerals_keep_the_placeholder_minutes_without_a_prediction() {
        assert_eq!(Numerals::narrow(None).minutes.as_deref(), Some("-"));
    }

    #[test]
    fn full_numerals_keep_the_minutes_at_any_day_count() {
        assert_eq!(
            Numerals::of(countdown(1_458)).minutes.as_deref(),
            Some("59")
        );
    }
}

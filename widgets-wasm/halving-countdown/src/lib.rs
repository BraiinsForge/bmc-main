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

//! Halving Countdown widget: the countdown to the next Bitcoin halving,
//! plus the predicted date and blocks remaining.
//!
//! The data is nexus's server-computed `bitcoin/halving-prediction`, so the
//! widget just renders the prediction and carries no halving math of its own.

mod manifest_params;

// Pure countdown arithmetic — compiled for the wasm widget and for native
// unit tests, but not the native non-test lib build (where it would be dead).
#[cfg(any(target_arch = "wasm32", test))]
const SECS_PER_MINUTE: i64 = 60;
#[cfg(any(target_arch = "wasm32", test))]
const SECS_PER_HOUR: i64 = 3_600;
#[cfg(any(target_arch = "wasm32", test))]
const SECS_PER_DAY: i64 = 86_400;

/// Days / hours / minutes remaining, already floored into place-values.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Countdown {
    days: i64,
    hours: i64,
    minutes: i64,
}

/// Split a non-negative second count into whole days, hours, and minutes.
/// Seconds are dropped — the widget only shows minute resolution.
#[cfg(any(target_arch = "wasm32", test))]
fn decompose(total_seconds: i64) -> Countdown {
    let total = total_seconds.max(0);
    Countdown {
        days: total / SECS_PER_DAY,
        hours: (total % SECS_PER_DAY) / SECS_PER_HOUR,
        minutes: (total % SECS_PER_HOUR) / SECS_PER_MINUTE,
    }
}

/// Blocks between the current tip and the next-halving target block.
#[cfg(any(target_arch = "wasm32", test))]
fn blocks_remaining(current_height: u32, target_block: u32) -> u32 {
    target_block.saturating_sub(current_height)
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::{Countdown, blocks_remaining, decompose, manifest_params};
    use std::cell::RefCell;

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render code uses many SDK exports and macros in one file"
    )]
    use bmc_wasm_sdk::*;

    const NEXUS_URL: &str = "https://nexus.braiinsforge.com/api/v1/data/bitcoin/halving-prediction";
    /// Poll cadence for fresh block data; the countdown itself is recomputed
    /// locally against the device clock on every frame.
    const REFRESH_MS: u32 = 60_000;
    /// Redraw cadence so the minutes place stays fresh between polls.
    const TICK_MS: u32 = 30_000;

    const NOT_AVAILABLE: &str = "-";
    const BITCOIN_GENESIS_UNIX: i64 = 1_231_006_505;

    // Round numerals are sized so the worst-case `DDDD:HH:MM` row
    // (four-digit days) clears the circular cutout when centered.
    const ROUND_TITLE_PX: u32 = 24;
    const ROUND_NUMERAL_PX: u32 = 64;
    const ROUND_LABEL_PX: u32 = 20;
    const ROUND_GAP_PX: f32 = 12.0;

    const TILE_BG: Color = Color::from_hex(0x0F_0F_0F);
    const TITLE_COLOR: Color = GRAY_60;
    const NUMERAL_COLOR: Color = WHITE;
    const LABEL_COLOR: Color = GRAY_60;
    const TILE_VALUE_COLOR: Color = WHITE;
    const TILE_SUB_COLOR: Color = GRAY_60;

    /// Font sizes for the three text rows of a bottom tile.
    #[derive(Clone, Copy)]
    struct TileSizes {
        label: u32,
        value: u32,
        sub: u32,
    }

    /// Which layout a size renders: a single compact countdown tile
    /// (Small/Medium and the round face), or the countdown plus the
    /// predicted-date and blocks-remaining tiles (Large/Full).
    #[derive(Clone, Copy)]
    enum Layout {
        Compact,
        Tiled(TileSizes),
    }

    struct SizeParams {
        layout: Layout,
        title: u32,
        numeral: u32,
        label: u32,
        padding: f32,
        gap: f32,
    }

    const FULL: SizeParams = SizeParams {
        layout: Layout::Tiled(TileSizes {
            label: 24,
            value: 40,
            sub: 24,
        }),
        title: 32,
        numeral: 120,
        label: 28,
        padding: 16.0,
        gap: 12.0,
    };
    const LARGE: SizeParams = SizeParams {
        layout: Layout::Tiled(TileSizes {
            label: 22,
            value: 36,
            sub: 22,
        }),
        title: 28,
        numeral: 96,
        label: 26,
        padding: 12.0,
        gap: 8.0,
    };
    const MEDIUM: SizeParams = SizeParams {
        layout: Layout::Compact,
        title: 26,
        numeral: 96,
        label: 24,
        padding: 8.0,
        gap: 20.0,
    };
    const SMALL: SizeParams = SizeParams {
        layout: Layout::Compact,
        title: 20,
        numeral: 48,
        label: 18,
        padding: 8.0,
        gap: 10.0,
    };

    fn size_params(variant: SizeVariant) -> &'static SizeParams {
        match variant {
            SizeVariant::Full => &FULL,
            SizeVariant::Large => &LARGE,
            SizeVariant::Medium => &MEDIUM,
            SizeVariant::Small => &SMALL,
        }
    }

    impl SizeParams {
        /// Downscale every font/spacing by `fit` (`WidgetSize::fit`): the
        /// rectangular viewport's ratio to the variant's canonical box, 1.0 at
        /// or above canonical size and smaller below it. Only the rectangular
        /// layouts call this; the round face scales itself in `view_round`.
        fn scaled(&self, fit: f32) -> Self {
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

    fn font_weight(style: manifest_params::NumbersFontStyle) -> FontWeight {
        use manifest_params::NumbersFontStyle;
        match style {
            NumbersFontStyle::Regular => FontWeight::REGULAR,
            NumbersFontStyle::SemiBold => FontWeight::SEMIBOLD,
            NumbersFontStyle::Bold => FontWeight::BOLD,
        }
    }

    /// Server-computed halving prediction, plus the lazily-formatted predicted
    /// date/time (cleared by `on_system_update` when locale/timezone changes).
    struct Prediction {
        current_height: u32,
        target_block: u32,
        predicted_unix: i64,
        formatted: Option<(String, String)>,
    }

    enum State {
        Loading,
        Loaded(Prediction),
        Error,
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "signature must match the poll Build fn pointer, which returns Option"
    )]
    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        Some(FetchSpec::get(NEXUS_URL))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let _ = register_poll(
            build_request,
            on_prediction,
            PollConfig {
                interval_ms: Some(REFRESH_MS),
                debounce_ms: 0,
                ..Default::default()
            },
        );
    }

    fn parse_prediction(response: &FetchResponse) -> Option<Prediction> {
        let json = response.json();
        let current_raw = json.i64("/data/current/block_height")?;
        let target_raw = json.i64("/data/next/block_height")?;
        let current_height = u32::try_from(current_raw).ok()?;
        let target_block = u32::try_from(target_raw).ok()?;
        if target_block == 0 {
            return None;
        }

        // Prefer the RFC3339 instant (stable across our local clock); fall back
        // to `next.delta` (seconds-from-now, recomputed server-side per read).
        let predicted_unix = json
            .str("/data/next/timestamp")
            .and_then(|ts| parse_date(&ts))
            .or_else(|| {
                json.i64("/data/next/delta")
                    .and_then(|delta| SystemTime::now().unix_secs.checked_add(delta))
            })?;
        if predicted_unix < BITCOIN_GENESIS_UNIX {
            return None;
        }

        Some(Prediction {
            current_height,
            target_block,
            predicted_unix,
            formatted: None,
        })
    }

    fn on_prediction(handle: PollHandle, response: &FetchResponse) {
        let parsed = if response.ok() {
            let p = parse_prediction(response);
            if p.is_none() {
                log_warn!("halving: 2xx payload missing/invalid prediction fields");
            }
            p
        } else {
            log_debug!("halving: fetch failed (status {})", response.status);
            None
        };

        if let Some(prediction) = parsed {
            STATE.with(|s| *s.borrow_mut() = State::Loaded(prediction));
        } else {
            // A 2xx with an unusable payload isn't a failure to the poll engine
            // (it reschedules off the HTTP status), so ask for a retry rather
            // than waiting the full interval. A non-2xx reschedules on its own.
            if response.ok() {
                handle.retry();
            }
            STATE.with(|s| {
                if matches!(&*s.borrow(), State::Loading) {
                    *s.borrow_mut() = State::Error;
                }
            });
        }
        request_frame();
    }

    /// Everything the render needs for a Loaded state, computed once per frame.
    struct DisplayData {
        countdown: Countdown,
        predicted_date: String,
        predicted_time: String,
        blocks_remaining: u32,
        target_block: String,
    }

    fn display_data() -> Option<DisplayData> {
        let now = SystemTime::now();
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let State::Loaded(pred) = &mut *state else {
                return None;
            };
            let total_seconds = pred.predicted_unix - now.unix_secs;
            let (predicted_date, predicted_time) = formatted_prediction(pred);
            Some(DisplayData {
                countdown: decompose(total_seconds),
                predicted_date,
                predicted_time,
                blocks_remaining: blocks_remaining(pred.current_height, pred.target_block),
                target_block: format_number!(f64::from(pred.target_block), 0),
            })
        })
    }

    fn formatted_prediction(pred: &mut Prediction) -> (String, String) {
        if let Some(cached) = &pred.formatted {
            return cached.clone();
        }
        let at = SystemTime {
            unix_secs: pred.predicted_unix,
        };
        let tz = system::current().timezone().map(Tz::from_runtime);

        let date = format_date(
            at,
            FormatDateOpts {
                timezone: tz.clone(),
                ..FormatDateOpts::default()
            },
        );
        let mut time = format_time(
            at,
            FormatTimeOpts {
                timezone: tz.clone(),
                ..FormatTimeOpts::default()
            },
        );
        // Host formatting returns empty for an out-of-range instant; fall back to `-`.
        if date.is_empty() || time.is_empty() {
            let na = (NOT_AVAILABLE.to_owned(), NOT_AVAILABLE.to_owned());
            pred.formatted = Some(na.clone());
            return na;
        }
        if let Some(meridiem) = format::meridiem(at, tz.as_ref()) {
            time.push(' ');
            time.push_str(&meridiem);
        }
        // Timezone caption (e.g. "Prague (+2)") — same convention as the clock widget.
        time.push(' ');
        format::push_tz_caption(&mut time, &format::resolve_tz_for_label(None, at.unix_secs));

        let formatted = (date, time);
        pred.formatted = Some(formatted.clone());
        formatted
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let weight = font_weight(manifest_params::Params::current().numbers_font_style);
        let data = display_data();

        let root = if matches!(widget_viewport().shape, ViewportShape::Round) {
            // The tiled/compact rectangular layouts pin content to the edges,
            // which the circular cutout clips; the round face centers a scaled
            // title · countdown · labels stack instead.
            view_round(ws, weight, data.as_ref())
        } else {
            let size = size_params(ws.variant).scaled(ws.fit());
            match size.layout {
                Layout::Compact => view_compact(&size, weight, data.as_ref()),
                Layout::Tiled(tiles) => view_tiles(&size, tiles, weight, data.as_ref()),
            }
        };

        let _ = render_ui(ws.width, ws.height, root);
        request_frame_after(TICK_MS);
    }

    /// Two-digit zero-padded, via the SDK's no-fmt writer (widget code must
    /// avoid `core::fmt` — see the `no-fmt-in-wasm` check).
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

    /// The three countdown numerals as display strings, or `-` when no data.
    fn numerals(data: Option<&DisplayData>) -> (String, String, String) {
        match data {
            Some(d) => (
                int_string(d.countdown.days),
                pad2(d.countdown.hours),
                pad2(d.countdown.minutes),
            ),
            None => (
                NOT_AVAILABLE.to_string(),
                NOT_AVAILABLE.to_string(),
                NOT_AVAILABLE.to_string(),
            ),
        }
    }

    fn title(size: &SizeParams) -> Node {
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
    fn column(value: impl Into<String>, name: &str, size: &SizeParams, weight: FontWeight) -> Node {
        col(
            props!(gap: 4.0, cross_align: CrossAlign::Center),
            [numeral(value, size, weight), label(name, size)],
        )
    }

    fn colon(size: &SizeParams, weight: FontWeight) -> Node {
        numeral(":", size, weight)
    }

    /// The `DD : HH : MM` row: per-segment numeral-over-label columns with
    /// colons between, top-aligned so the colons sit with the numerals and
    /// each label stays centered under its own number.
    fn countdown_columns(
        days: String,
        hours: String,
        minutes: String,
        size: &SizeParams,
        weight: FontWeight,
        gap: f32,
    ) -> Node {
        row(
            props!(gap: gap, cross_align: CrossAlign::Start),
            [
                column(days, "Days", size, weight),
                colon(size, weight),
                column(hours, "Hours", size, weight),
                colon(size, weight),
                column(minutes, "Min.", size, weight),
            ],
        )
    }

    /// Tiled layout (Large/Full): title + countdown on top, predicted-date and
    /// blocks-remaining tiles below.
    fn view_tiles(
        size: &SizeParams,
        tiles: TileSizes,
        weight: FontWeight,
        data: Option<&DisplayData>,
    ) -> Node {
        let (days, hours, minutes) = numerals(data);
        let countdown = countdown_columns(days, hours, minutes, size, weight, size.gap * 2.0);

        let top = col(
            props!(background: TILE_BG, flex: 3.0),
            [center(
                props!(flex: 1.0),
                [col(
                    props!(gap: size.gap + 4.0, cross_align: CrossAlign::Center),
                    [title(size), countdown],
                )],
            )],
        );

        let bottom = row(
            props!(gap: size.gap, flex: 2.0),
            [
                info_tile(
                    "Predicted Date",
                    data.map(|d| d.predicted_date.clone()),
                    data.map(|d| d.predicted_time.clone()),
                    tiles,
                ),
                info_tile(
                    "Blocks Remaining",
                    data.map(|d| format_number!(f64::from(d.blocks_remaining), 0)),
                    data.map(|d| {
                        let mut s = String::from("Target #");
                        s.push_str(&d.target_block);
                        s
                    }),
                    tiles,
                ),
            ],
        );

        col(
            props!(background: BLACK, padding: size.padding, gap: size.gap),
            [top, bottom],
        )
    }

    /// A bottom tile: caption, big value, sub-line. Value/sub fall back to
    /// `-` / empty when no data.
    fn info_tile(
        caption: &str,
        value: Option<String>,
        sub: Option<String>,
        tiles: TileSizes,
    ) -> Node {
        let value = value.unwrap_or_else(|| NOT_AVAILABLE.to_string());
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

    /// Compact layout (Small/Medium): a single tile with the title on top and
    /// the countdown numerals over their labels, centered.
    fn view_compact(size: &SizeParams, weight: FontWeight, data: Option<&DisplayData>) -> Node {
        let (days, hours, minutes) = numerals(data);
        let countdown = countdown_columns(days, hours, minutes, size, weight, size.gap);

        let tile = col(
            props!(background: TILE_BG, padding: 12.0, flex: 1.0),
            [title(size), center(props!(flex: 1.0), [countdown])],
        );

        col(props!(background: BLACK, padding: size.padding), [tile])
    }

    /// Round layout: a vertically-centered title · countdown · labels stack,
    /// scaled to sit within the circular cutout.
    fn view_round(ws: WidgetSize, weight: FontWeight, data: Option<&DisplayData>) -> Node {
        let scale = ws.round_scale();
        let size = SizeParams {
            layout: Layout::Compact,
            title: scale_font(ROUND_TITLE_PX, scale),
            numeral: scale_font(ROUND_NUMERAL_PX, scale),
            label: scale_font(ROUND_LABEL_PX, scale),
            padding: 0.0,
            gap: ROUND_GAP_PX * scale,
        };
        let (days, hours, minutes) = numerals(data);
        let countdown = countdown_columns(days, hours, minutes, &size, weight, size.gap);

        col(
            props!(background: BLACK),
            [center(
                props!(flex: 1.0),
                [col(
                    props!(gap: size.gap, cross_align: CrossAlign::Center),
                    [title(&size), countdown],
                )],
            )],
        )
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        STATE.with(|s| {
            if let State::Loaded(pred) = &mut *s.borrow_mut() {
                pred.formatted = None;
            }
        });
        request_frame();
    }
}

#[cfg(test)]
mod tests {
    use super::{blocks_remaining, decompose};

    #[test]
    fn decompose_splits_place_values() {
        // 2 days, 3 hours, 4 minutes, 30 seconds (seconds dropped).
        let total = 2 * 86_400 + 3 * 3_600 + 4 * 60 + 30;
        let cd = decompose(total);
        assert_eq!(cd.days, 2);
        assert_eq!(cd.hours, 3);
        assert_eq!(cd.minutes, 4);
    }

    #[test]
    fn decompose_clamps_negative_to_zero() {
        let cd = decompose(-500);
        assert_eq!((cd.days, cd.hours, cd.minutes), (0, 0, 0));
    }

    #[test]
    fn decompose_large_span() {
        // ~631 days out (a fresh halving era), like the live nexus payload.
        let cd = decompose(54_533_098);
        assert_eq!(cd.days, 631);
        assert_eq!(cd.hours, 4);
        assert_eq!(cd.minutes, 4);
    }

    #[test]
    fn blocks_remaining_is_target_minus_tip() {
        assert_eq!(blocks_remaining(959_111, 1_050_000), 90_889);
    }

    #[test]
    fn blocks_remaining_saturates_past_target() {
        assert_eq!(blocks_remaining(1_050_001, 1_050_000), 0);
    }
}

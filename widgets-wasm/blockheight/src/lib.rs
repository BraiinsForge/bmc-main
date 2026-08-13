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

//! Blockheight widget — Bitcoin block height + timestamp, four sizes.

mod manifest_params;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::manifest_params;
    use std::cell::RefCell;

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render code uses many SDK exports and macros in one file"
    )]
    use bmc_wasm_sdk::*;

    const BLOCK_HEIGHT_API_URL: &str =
        "https://nexus.braiinsforge.com/api/v1/data/bitcoin/block/latest";
    const REFRESH_MS: u32 = 60_000;

    /// Bitcoin genesis block time (2009-01-03).
    const BITCOIN_GENESIS_UNIX: i64 = 1_231_006_505;

    const NOT_AVAILABLE: &str = "--";

    #[derive(Clone, Copy)]
    struct SizeParams {
        number_font_size: u32,
        timestamp_font_size: u32,
        caption_font_size: u32,
        padding_left: f32,
        padding_top: f32,
        /// Gap from the bottom edge to the last timestamp line.
        /// The compact faces share `nameday`'s 16px date inset.
        /// Fullscreen keeps the timestamp up near the numeral.
        timestamp_pad_bottom: f32,
    }

    const SMALL: SizeParams = SizeParams {
        number_font_size: 64,
        timestamp_font_size: 24,
        caption_font_size: 18,
        padding_left: 16.0,
        padding_top: 8.0,
        timestamp_pad_bottom: 16.0,
    };
    const MEDIUM: SizeParams = SizeParams {
        number_font_size: 96,
        timestamp_font_size: 24,
        caption_font_size: 18,
        padding_left: 16.0,
        padding_top: 8.0,
        timestamp_pad_bottom: 16.0,
    };
    const LARGE: SizeParams = SizeParams {
        number_font_size: 120,
        timestamp_font_size: 32,
        caption_font_size: 22,
        padding_left: 16.0,
        padding_top: 8.0,
        timestamp_pad_bottom: 16.0,
    };
    const FULL: SizeParams = SizeParams {
        number_font_size: 200,
        timestamp_font_size: 48,
        caption_font_size: 32,
        padding_left: 24.0,
        padding_top: 16.0,
        timestamp_pad_bottom: 60.0,
    };

    fn size_params(variant: SizeVariant) -> &'static SizeParams {
        match variant {
            SizeVariant::Full => &FULL,
            SizeVariant::Large => &LARGE,
            SizeVariant::Medium => &MEDIUM,
            SizeVariant::Small => &SMALL,
        }
    }

    const CUBE_ICON: Svg = include_svg!("assets/cube.svg");
    const CUBE_PX: f32 = 24.0;
    const HEADER_GAP_PX: f32 = 12.0;
    const HEADER_FONT_PX: u32 = 24;
    /// Header weight and color track `nameday` / `random-facts`
    /// rather than this widget's Figma frame (semibold Gray/40).
    /// The faces sit side by side on the Deck, so they have to match each other.
    const HEADER_WEIGHT: FontWeight = FontWeight::BOLD;
    const HEADER_COLOR: Color = GRAY_60;
    const HEIGHT_COLOR: Color = WHITE;
    const TIMESTAMP_COLOR: Color = GRAY_60;
    const FOUND_AT_CAPTION: &str = "Found at";
    const CAPTION_GAP_PX: f32 = 4.0;

    // Round sizes stay conservative so a grouped 7-digit height
    // ("9 999 999") clears the circular cutout when centered.
    const ROUND_NUMBER_PX: u32 = 80;
    const ROUND_TIMESTAMP_PX: u32 = 24;
    const ROUND_CAPTION_PX: u32 = 18;
    const ROUND_STACK_GAP_PX: f32 = 16.0;

    fn timestamp_block(caption_size: u32, timestamp_size: u32, gap: f32) -> Node {
        col(
            props!(gap: gap, cross_align: CrossAlign::Center),
            [
                text(
                    FOUND_AT_CAPTION,
                    style!(
                        size: caption_size,
                        weight: FontWeight::REGULAR,
                        color: TIMESTAMP_COLOR,
                        // Tight line box keeps the caption on its date;
                        // the default 1.4 drifts them apart as the font grows.
                        line_height: 0.8,
                    ),
                ),
                text(
                    format_timestamp(),
                    style!(size: timestamp_size, weight: FontWeight::REGULAR, color: TIMESTAMP_COLOR),
                ),
            ],
        )
    }

    fn font_weight(style: manifest_params::NumbersFontStyle) -> FontWeight {
        use manifest_params::NumbersFontStyle;
        match style {
            NumbersFontStyle::Regular => FontWeight::REGULAR,
            NumbersFontStyle::SemiBold => FontWeight::SEMIBOLD,
            NumbersFontStyle::Bold => FontWeight::BOLD,
        }
    }

    struct BlockData {
        height: u32,
        /// Block header time, unix seconds (nexus normalizes it server-side).
        timestamp_unix: i64,
        /// Cached `format_date + ", " + format_time` against the host snapshot.
        /// Cleared whenever that snapshot may have changed.
        formatted_timestamp: Option<String>,
    }

    enum State {
        Loading,
        Loaded(BlockData),
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
        Some(FetchSpec::get(BLOCK_HEIGHT_API_URL))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let _ = register_poll(
            build_request,
            on_block_data,
            PollConfig {
                interval_ms: Some(REFRESH_MS),
                debounce_ms: 0,
                ..Default::default()
            },
        );
    }

    fn on_block_data(handle: PollHandle, response: &FetchResponse) {
        let outcome = if response.ok() {
            let json = response.json();
            let raw_height = json.i64("/data/height");
            let timestamp_unix = json.i64("/data/time");
            match (raw_height, timestamp_unix) {
                (None, _) | (_, None) => {
                    log_warn!("blockheight: payload missing height or time");
                    None
                }
                (Some(raw), _) if u32::try_from(raw).is_err() => {
                    log_warn!("blockheight: height {raw} out of u32 range; ignoring payload");
                    None
                }
                (_, Some(time)) if time < BITCOIN_GENESIS_UNIX => {
                    log_warn!("blockheight: block time {time} precedes genesis; ignoring payload");
                    None
                }
                (Some(raw), Some(timestamp_unix)) => {
                    let height = u32::try_from(raw)
                        .expect("BUG: u32::try_from re-checked after explicit Err branch above");
                    Some(BlockData {
                        height,
                        timestamp_unix,
                        formatted_timestamp: None,
                    })
                }
            }
        } else {
            log_debug!("blockheight: fetch failed (status {})", response.status);
            None
        };

        if let Some(data) = outcome {
            STATE.with(|s| *s.borrow_mut() = State::Loaded(data));
        } else {
            // A 2xx with an unusable payload doesn't count as a failure to the
            // poll engine, which reschedules off the HTTP status, so ask it to
            // retry after retry_ms rather than waiting the full refresh interval.
            // A non-2xx already reschedules as a failure on its own.
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

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let ws = widget_size();
        let params = manifest_params::Params::current();
        let root = if matches!(widget_viewport().shape, ViewportShape::Round) {
            view_round(ws, &params)
        } else {
            view_rect(size_params(ws.variant), &params)
        };
        let _ = render_ui(ws.width, ws.height, root);
    }

    /// Rectangular layout: header pinned top-left, the height numeral centered
    /// on the face, and the "Found at" timestamp pinned near the bottom edge.
    fn view_rect(size: &SizeParams, params: &manifest_params::Params) -> Node {
        let header_overlay = row(
            props!(
                inset_top: size.padding_top,
                inset_left: size.padding_left,
                gap: HEADER_GAP_PX,
                cross_align: CrossAlign::Center,
            ),
            [
                canvas(
                    props!(width: CUBE_PX, height: CUBE_PX),
                    vec![Draw::svg(
                        0.0,
                        0.0,
                        CUBE_PX,
                        CUBE_PX,
                        &CUBE_ICON,
                        HEADER_COLOR,
                    )],
                ),
                text(
                    "Block Height",
                    style!(size: HEADER_FONT_PX, weight: HEADER_WEIGHT, color: HEADER_COLOR),
                ),
            ],
        );

        let number = text(
            format_height(),
            style!(
                size: size.number_font_size,
                weight: font_weight(params.numbers_font_style),
                color: HEIGHT_COLOR,
                family: FontFamily::DeckSans,
                // Tight line box: the numerals have no descenders,
                // so the default 1.4 multiplier would leave the box mostly empty.
                line_height: 0.8,
            ),
        );

        let mut layers = vec![header_overlay, center(props!(flex: 1.0), [number])];

        // Anchored to the bottom edge, not to the numeral, which owns the center.
        // Pinning by the block's bottom holds the date as the caption grows.
        if params.show_timestamp {
            layers.push(center(
                props!(
                    inset_bottom: size.timestamp_pad_bottom,
                    inset_left: 0.0,
                    inset_right: 0.0,
                ),
                [timestamp_block(
                    size.caption_font_size,
                    size.timestamp_font_size,
                    CAPTION_GAP_PX,
                )],
            ));
        }

        col(props!(background: BLACK), layers)
    }

    /// Round layout: a vertically-centered header · height · timestamp stack.
    /// The rectangular layout's edge-pinned rows would spill past the circular
    /// cutout, so the round face centers everything and scales it to fit.
    fn view_round(ws: WidgetSize, params: &manifest_params::Params) -> Node {
        let scale = ws.round_scale();
        let icon_px = CUBE_PX * scale;

        let header = row(
            props!(gap: HEADER_GAP_PX * scale, cross_align: CrossAlign::Center),
            [
                canvas(
                    props!(width: icon_px, height: icon_px),
                    vec![Draw::svg(
                        0.0,
                        0.0,
                        icon_px,
                        icon_px,
                        &CUBE_ICON,
                        HEADER_COLOR,
                    )],
                ),
                text(
                    "Block Height",
                    style!(
                        size: scale_font(HEADER_FONT_PX, scale),
                        weight: HEADER_WEIGHT,
                        color: HEADER_COLOR,
                    ),
                ),
            ],
        );

        let height_node = text(
            format_height(),
            style!(
                size: scale_font(ROUND_NUMBER_PX, scale),
                weight: font_weight(params.numbers_font_style),
                color: HEIGHT_COLOR,
                family: FontFamily::DeckSans,
            ),
        );

        let mut stack: Vec<Node> = vec![header, height_node];
        if params.show_timestamp {
            stack.push(timestamp_block(
                scale_font(ROUND_CAPTION_PX, scale),
                scale_font(ROUND_TIMESTAMP_PX, scale),
                CAPTION_GAP_PX * scale,
            ));
        }

        col(
            props!(background: BLACK),
            [center(
                props!(flex: 1.0),
                [col(
                    props!(gap: ROUND_STACK_GAP_PX * scale, cross_align: CrossAlign::Center),
                    stack,
                )],
            )],
        )
    }

    fn format_height() -> String {
        STATE.with(|s| match &*s.borrow() {
            State::Loaded(data) => format_number!(f64::from(data.height), 0),
            State::Loading | State::Error => NOT_AVAILABLE.to_string(),
        })
    }

    fn format_timestamp() -> String {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let State::Loaded(data) = &mut *state else {
                return NOT_AVAILABLE.to_string();
            };
            if let Some(cached) = &data.formatted_timestamp {
                return cached.clone();
            }
            let formatted = format_timestamp_str(data.timestamp_unix);
            data.formatted_timestamp = Some(formatted.clone());
            formatted
        })
    }

    fn format_timestamp_str(unix_secs: i64) -> String {
        let now = SystemTime { unix_secs };

        let tz = system::current().timezone().map(Tz::from_runtime);

        let date_str = format_date(
            now,
            FormatDateOpts {
                timezone: tz.clone(),
                ..FormatDateOpts::default()
            },
        );
        let time_str = format_time(
            now,
            FormatTimeOpts {
                timezone: tz,
                ..FormatTimeOpts::default()
            },
        );
        if date_str.is_empty() || time_str.is_empty() {
            return NOT_AVAILABLE.to_string();
        }
        let mut out = String::with_capacity(date_str.len() + 2 + time_str.len());
        out.push_str(&date_str);
        out.push_str(", ");
        out.push_str(&time_str);
        out
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        STATE.with(|s| {
            if let State::Loaded(data) = &mut *s.borrow_mut() {
                data.formatted_timestamp = None;
            }
        });
        request_frame();
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! Blockheight widget — Bitcoin block height + timestamp, four sizes.
//! Visual parity with `bmc-display/ui/widgets/categories/block-height.slint`
//! on `bmc/stable-26.02`.

mod manifest_params;

use std::cell::RefCell;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

// `currency=usd` is required by the API; without it the endpoint returns 400.
const BLOCK_HEIGHT_API_URL: &str = "https://public-api.braiins.com/v2/blocks?limit=1&currency=usd";
const REFRESH_MS: u32 = 60_000;
const RETRY_MS: u32 = 10_000;

const NOT_AVAILABLE: &str = "--";

#[derive(Clone, Copy)]
struct SizeParams {
    number_font_size: u32,
    timestamp_font_size: u32,
    padding_left: f32,
    padding_top: f32,
    padding_bottom: f32,
}

const SMALL: SizeParams = SizeParams {
    number_font_size: 64,
    timestamp_font_size: 24,
    padding_left: 16.0,
    padding_top: 8.0,
    padding_bottom: 16.0,
};
const MEDIUM: SizeParams = SizeParams {
    number_font_size: 96,
    timestamp_font_size: 24,
    padding_left: 16.0,
    padding_top: 8.0,
    padding_bottom: 16.0,
};
const LARGE: SizeParams = SizeParams {
    number_font_size: 120,
    timestamp_font_size: 32,
    padding_left: 16.0,
    padding_top: 8.0,
    padding_bottom: 16.0,
};
const FULL: SizeParams = SizeParams {
    number_font_size: 200,
    timestamp_font_size: 48,
    padding_left: 24.0,
    padding_top: 16.0,
    padding_bottom: 60.0,
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
const HEADER_COLOR: Color = GRAY_60;
const HEIGHT_COLOR: Color = WHITE;
const TIMESTAMP_COLOR: Color = GRAY_60;

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
    timestamp_utc: String,
}

enum State {
    Loading,
    Loaded(BlockData),
    Error,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
}

/// Queue the next fetch. The widget is strictly single-flight: at most one
/// fetch is queued+in-flight at a time, and this function is only called
/// from `init` (before any fetch is in flight) and from `on_block_data`
/// (immediately after the previous fetch completed, so its slot is freed).
/// A rejection from the host therefore implies `max_fetches == 0` or a
/// runtime bug — surface it loudly rather than silently freezing the widget.
fn schedule_fetch(delay_ms: Option<u32>) {
    let queued = match delay_ms {
        Some(ms) => fetch_after(ms, BLOCK_HEIGHT_API_URL, None, on_block_data),
        None => fetch(BLOCK_HEIGHT_API_URL, None, on_block_data),
    };
    queued.expect(
        "BUG: blockheight is single-flight; host fetch budget should never be exhausted at schedule time",
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    schedule_fetch(None);
}

fn on_block_data(response: &FetchResponse) {
    let outcome = if response.ok() {
        let json = response.json();
        let raw_height = json.i64("/0/height");
        let timestamp = json.str("/0/timestamp");
        match (raw_height, timestamp) {
            (None, _) | (_, None) => {
                log_warn!("blockheight: payload missing height or timestamp");
                None
            }
            (Some(raw), _) if u32::try_from(raw).is_err() => {
                log_warn!("blockheight: height {raw} out of u32 range; ignoring payload");
                None
            }
            (Some(raw), Some(timestamp_utc)) => {
                let height = u32::try_from(raw)
                    .expect("BUG: u32::try_from re-checked after explicit Err branch above");
                Some(BlockData {
                    height,
                    timestamp_utc,
                })
            }
        }
    } else {
        log_warn!("blockheight: fetch failed (status {})", response.status);
        None
    };

    let next_delay = if let Some(data) = outcome {
        STATE.with(|s| *s.borrow_mut() = State::Loaded(data));
        REFRESH_MS
    } else {
        STATE.with(|s| {
            if matches!(&*s.borrow(), State::Loading) {
                *s.borrow_mut() = State::Error;
            }
        });
        RETRY_MS
    };

    request_frame();
    schedule_fetch(Some(next_delay));
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width,
        height,
        variant,
    } = widget_size();
    let size = size_params(variant);
    let params = manifest_params::Params::current();

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
                style!(
                    size: HEADER_FONT_PX,
                    weight: FontWeight::REGULAR,
                    color: HEADER_COLOR,
                ),
            ),
        ],
    );

    let height_node = center(
        props!(flex: 1.0),
        [text(
            format_height(),
            style!(
                size: size.number_font_size,
                weight: font_weight(params.numbers_font_style),
                color: HEIGHT_COLOR,
                family: FontFamily::DeckSans,
            ),
        )],
    );

    let mut root_children: Vec<Node> = vec![header_overlay, height_node];

    if params.show_timestamp {
        root_children.push(center(
            props!(
                inset_bottom: size.padding_bottom,
                inset_left: 0.0,
                inset_right: 0.0,
            ),
            [text(
                format_timestamp(),
                style!(
                    size: size.timestamp_font_size,
                    weight: FontWeight::REGULAR,
                    color: TIMESTAMP_COLOR,
                ),
            )],
        ));
    }

    let root = col(props!(background: BLACK), root_children);

    let _ = render_ui(width, height, root);
}

fn format_height() -> String {
    STATE.with(|s| match &*s.borrow() {
        State::Loaded(data) => format_number!(f64::from(data.height), 0),
        State::Loading | State::Error => NOT_AVAILABLE.to_string(),
    })
}

fn format_timestamp() -> String {
    NOT_AVAILABLE.to_string()
}

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
    let size = widget_size();
    let root = col(props!(background: BLACK), Vec::<Node>::new());
    let _ = render_ui(size.width, size.height, root);
}

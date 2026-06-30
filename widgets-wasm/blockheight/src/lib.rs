// Copyright (C) 2026  Braiins Systems s.r.o.

//! Blockheight widget — Bitcoin block height + timestamp, four sizes.
//! Visual parity with `bmc-display/ui/widgets/categories/block-height.slint`
//! on `bmc/stable-26.02`.

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

    // `currency=usd` is required by the API; without it the endpoint returns 400.
    const BLOCK_HEIGHT_API_URL: &str =
        "https://public-api.braiins.com/v2/blocks?limit=1&currency=usd";
    const REFRESH_MS: u32 = 60_000;

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
        /// `format_date + ", " + format_time` against the current host
        /// snapshot. `None` until first computed in `render`, and cleared by
        /// `on_system_update` when the snapshot may have changed.
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
                        formatted_timestamp: None,
                    })
                }
            }
        } else {
            log_warn!("blockheight: fetch failed (status {})", response.status);
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
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            let State::Loaded(data) = &mut *state else {
                return NOT_AVAILABLE.to_string();
            };
            if let Some(cached) = &data.formatted_timestamp {
                return cached.clone();
            }
            let formatted = format_timestamp_str(&data.timestamp_utc);
            data.formatted_timestamp = Some(formatted.clone());
            formatted
        })
    }

    fn format_timestamp_str(raw_utc: &str) -> String {
        let mut rfc3339 = String::with_capacity(raw_utc.len() + 1);
        rfc3339.push_str(raw_utc);
        rfc3339.push('Z');

        let Some(unix_secs) = parse_date(&rfc3339) else {
            return NOT_AVAILABLE.to_string();
        };
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

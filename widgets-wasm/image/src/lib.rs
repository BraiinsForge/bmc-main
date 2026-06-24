// Copyright (C) 2026  Braiins Systems s.r.o.

//! Image widget — fetches an image from a URL and displays it fitted to the
//! widget viewport.

mod manifest_params;
#[cfg(any(target_arch = "wasm32", test))]
pub mod render;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::{manifest_params, render};
    use std::cell::{Cell, RefCell};

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render uses many SDK exports"
    )]
    use bmc_wasm_sdk::*;

    const RETRY_MS: u32 = 10_000;
    const DEBOUNCE_MS: u32 = 300;
    /// jpeg-decoder DCT-scales to 1/8 per axis, tolerating 64× more source pixels.
    const JPEG_SCALE_HEADROOM: u64 = 64;

    /// Displayed image; set() evicts the previous.
    static IMAGE: BitmapSlot = BitmapSlot::new("image");

    enum State {
        Loading,
        Loaded { bitmap: BitmapId, aspect: f32 },
        LoadFailed,
        TooLarge,
        BadImage,
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
        static POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        // Held image; stale after a failed refresh.
        static STALE: Cell<bool> = const { Cell::new(false) };
    }

    /// Refresh interval (ms); fixed at registration.
    fn refresh_interval_ms() -> u32 {
        let secs = manifest_params::Params::current().refresh_seconds.max(1);
        u32::try_from(secs).unwrap_or(u32::MAX).saturating_mul(1000)
    }

    /// Largest source the host can bound for this body's format.
    fn max_source_pixels(body: &[u8]) -> u64 {
        let cap = u64::from(host::max_image_pixels());
        if body.starts_with(&[0xFF, 0xD8, 0xFF]) {
            cap * JPEG_SCALE_HEADROOM
        } else {
            cap
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let handle = register_poll(
            build_request,
            on_image,
            PollConfig {
                interval_ms: Some(refresh_interval_ms()),
                retry_ms: RETRY_MS,
                debounce_ms: DEBOUNCE_MS,
                enabled: true,
            },
        );
        POLL.with(|p| p.set(Some(handle)));
    }

    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        let url = manifest_params::Params::current().url;
        let url = url.trim();
        if url.is_empty() {
            return None;
        }
        Some(FetchSpec::get(url.to_owned()))
    }

    fn on_image(handle: PollHandle, response: &FetchResponse) {
        let has_image = STATE.with(|s| matches!(&*s.borrow(), State::Loaded { .. }));

        // Probe dims before register so oversized skips the upload.
        let error: Option<(State, bool)> = if !response.ok() || response.body().is_empty() {
            Some((State::LoadFailed, true))
        } else {
            match host::image_dimensions(response.body()) {
                None => Some((State::BadImage, false)),
                Some((w, h))
                    if u64::from(w) * u64::from(h) > max_source_pixels(response.body()) =>
                {
                    Some((State::TooLarge, false))
                }
                Some((w, h)) => {
                    let size = widget_size();
                    match IMAGE.set_fit(response.body(), size.width, size.height) {
                        Some(bitmap) => {
                            let aspect = render::aspect_of(w, h);
                            STATE.with(|s| *s.borrow_mut() = State::Loaded { bitmap, aspect });
                            STALE.with(|s| s.set(false));
                            None
                        }
                        None => Some((State::BadImage, false)),
                    }
                }
            }
        };

        if let Some((state, transient)) = error {
            if has_image {
                // Failed refresh: keep last good (stale); retry transient.
                STALE.with(|s| s.set(true));
                if transient {
                    handle.retry();
                }
            } else {
                STATE.with(|s| *s.borrow_mut() = state);
            }
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        if prev.as_ref().is_none_or(|p| p.url != cur.url) {
            // No evict: renderer imports trap outside render; set() replaces it.
            STATE.with(|s| *s.borrow_mut() = State::Loading);
            STALE.with(|s| s.set(false));
            POLL.with(|p| {
                if let Some(handle) = p.get() {
                    handle.set_enabled(true);
                    handle.invalidate();
                }
            });
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let size = widget_size();
        let url = manifest_params::Params::current().url;
        let node = if url.trim().is_empty() {
            render::message_view(render::CONFIGURE_URL, size)
        } else {
            STATE.with(|s| match &*s.borrow() {
                State::Loaded { bitmap, aspect } => {
                    let view = render::image_view(*bitmap, *aspect, size);
                    if STALE.with(Cell::get) {
                        render::with_stale_banner(view)
                    } else {
                        view
                    }
                }
                State::Loading => render::message_view(render::LOADING, size),
                State::LoadFailed => render::message_view(render::LOAD_FAILED, size),
                State::TooLarge => render::message_view(render::TOO_LARGE, size),
                State::BadImage => render::message_view(render::BAD_IMAGE, size),
            })
        };
        let _ = render_ui(size.width, size.height, node);
    }
}

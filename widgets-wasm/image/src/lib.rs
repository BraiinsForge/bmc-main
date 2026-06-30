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
    /// Menu clears itself this long after opening, untouched.
    const MENU_AUTO_DISMISS_MS: u32 = 10_000;

    /// Displayed image; set() evicts the previous.
    static IMAGE: BitmapSlot = BitmapSlot::new("image");

    enum State {
        Loading,
        Loaded { bitmap: BitmapId, aspect: f32 },
        LoadFailed,
        TooLarge,
        BadImage,
    }

    /// Outcome of restoring the cached image.
    enum Restore {
        /// Within TTL; carries ms until the next refresh.
        Fresh { remaining_ms: u32 },
        /// Restored but past the TTL — shown now, refreshing in the background.
        Stale,
        /// No usable bucket (missing, or a different URL/viewport) — must refetch.
        Miss,
    }

    /// In-flight decode: its job handle and the source image's aspect ratio.
    #[derive(Clone, Copy)]
    struct Pending {
        job: ImageJobId,
        aspect: f32,
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State::Loading) };
        static POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        // Held image; stale after a failed refresh.
        static STALE: Cell<bool> = const { Cell::new(false) };
        // Showing a cached image while a fresh fetch runs (wake from a past-TTL bucket).
        static REFRESHING: Cell<bool> = const { Cell::new(false) };
        // In-flight decode; results from superseded jobs are ignored.
        static PENDING: Cell<Option<Pending>> = const { Cell::new(None) };
        // Menu auto-dismiss countdown (ms); 0 = closed.
        static MENU_MS: Cell<u32> = const { Cell::new(0) };
        // First render restores from cache (init() has no renderer scope).
        static INITIAL_RESTORE: Cell<bool> = const { Cell::new(false) };
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
                enabled: false, // first render restores + schedules
            },
        );
        POLL.with(|p| p.set(Some(handle)));
        INITIAL_RESTORE.with(|f| f.set(true));
    }

    // {{width}}/{{height}} expand to the viewport pixels — 1:1 with the
    // released Slint widget so existing server URLs carry over unchanged.
    fn expanded_url() -> Option<String> {
        let url = manifest_params::Params::current().url;
        let url = url.trim();
        if url.is_empty() {
            return None;
        }
        let size = widget_size();
        Some(
            url.replace("{{width}}", &size.width.to_string())
                .replace("{{height}}", &size.height.to_string()),
        )
    }

    // Cache identity: the expanded URL plus the sizing mode,
    // so a URL, viewport, or sizing change is a distinct cached blob.
    fn cache_identity() -> Option<String> {
        let mut id = expanded_url()?;
        id.push('\u{1f}');
        id.push_str(
            manifest_params::Params::current()
                .sizing
                .as_manifest_value(),
        );
        Some(id)
    }

    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        expanded_url().map(FetchSpec::get)
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
                    let aspect = render::aspect_of(w, h);
                    // Identity = URL + viewport + sizing; the host stamps it for
                    // restore-on-wake (a distinct blob per sizing mode).
                    let identity = cache_identity().unwrap_or_default();
                    let cover =
                        manifest_params::Params::current().sizing == manifest_params::Sizing::Cover;
                    match IMAGE.set_fit(
                        response.body(),
                        size.width,
                        size.height,
                        cover,
                        identity.as_bytes(),
                        on_decoded,
                    ) {
                        Some(job) => {
                            PENDING.with(|p| p.set(Some(Pending { job, aspect })));
                            None
                        }
                        // Decode slots full — retry later.
                        None => Some((State::LoadFailed, true)),
                    }
                }
            }
        };

        if let Some((state, transient)) = error {
            if has_image {
                // Failed refresh: keep last good (stale), drop the overlay; retry transient.
                REFRESHING.with(|s| s.set(false));
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

    /// Async decode finished: swap to the new bitmap, or mark bad/stale on failure.
    fn on_decoded(job: ImageJobId, bitmap: Option<BitmapId>) {
        let Some(pending) = PENDING.with(Cell::get) else {
            return;
        };
        if pending.job != job {
            return;
        }
        // The refresh resolved either way — drop the overlay.
        REFRESHING.with(|r| r.set(false));
        match bitmap {
            Some(bitmap) => {
                STATE.with(|s| {
                    *s.borrow_mut() = State::Loaded {
                        bitmap,
                        aspect: pending.aspect,
                    };
                });
                STALE.with(|s| s.set(false));
            }
            None if STATE.with(|s| matches!(&*s.borrow(), State::Loaded { .. })) => {
                STALE.with(|s| s.set(true));
            }
            None => STATE.with(|s| *s.borrow_mut() = State::BadImage),
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        if prev
            .as_ref()
            .is_none_or(|p| p.url != cur.url || p.sizing != cur.sizing)
        {
            // No evict: renderer imports trap outside render; set() replaces it.
            STATE.with(|s| *s.borrow_mut() = State::Loading);
            STALE.with(|s| s.set(false));
            REFRESHING.with(|r| r.set(false));
            resume_polling();
        }
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_touch() {
        // The host only delivers touch to on_touch exporters; wake a frame to read it.
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_dormant() {
        // Stop polling off-screen; the host frees our texture.
        POLL.with(|p| {
            if let Some(handle) = p.get() {
                handle.set_enabled(false);
            }
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_wake() {
        INITIAL_RESTORE.with(|f| f.set(false)); // wake subsumes cold-start restore
        STALE.with(|s| s.set(false));
        restore_then_schedule();
        request_frame();
    }

    /// Restore from cache, then schedule the poll (fresh → fetch at TTL;
    /// stale/miss → now). Render-scope only — calls renderer imports.
    fn restore_then_schedule() {
        match cache_identity().map_or(Restore::Miss, |id| try_restore(&id)) {
            Restore::Fresh { remaining_ms } => {
                REFRESHING.with(|r| r.set(false));
                POLL.with(|p| {
                    if let Some(handle) = p.get() {
                        handle.enable_after(remaining_ms);
                    }
                });
            }
            Restore::Stale => {
                REFRESHING.with(|r| r.set(true));
                resume_polling();
            }
            Restore::Miss => {
                REFRESHING.with(|r| r.set(false));
                STATE.with(|s| *s.borrow_mut() = State::Loading);
                resume_polling();
            }
        }
    }

    /// Re-enable the refresh poll and fetch immediately.
    fn resume_polling() {
        POLL.with(|p| {
            if let Some(handle) = p.get() {
                handle.set_enabled(true);
                handle.invalidate();
            }
        });
    }

    fn menu_open() -> bool {
        MENU_MS.with(Cell::get) > 0
    }

    fn open_menu() {
        MENU_MS.with(|m| m.set(MENU_AUTO_DISMISS_MS));
    }

    fn close_menu() {
        MENU_MS.with(|m| m.set(0));
    }

    /// Re-fetch now, bypassing the TTL — the menu's Reload and error-retry path.
    fn force_reload() {
        let has_image = STATE.with(|s| matches!(&*s.borrow(), State::Loaded { .. }));
        if has_image {
            REFRESHING.with(|r| r.set(true));
        } else {
            STATE.with(|s| *s.borrow_mut() = State::Loading);
            STALE.with(|s| s.set(false));
        }
        resume_polling();
    }

    /// Re-register the cached image on an identity match, regardless of freshness.
    fn try_restore(current_identity: &str) -> Restore {
        let Some(stat) = cache::stat(IMAGE.name()) else {
            return Restore::Miss;
        };
        let Some((w, h, identity)) = parse_meta(&stat.metadata) else {
            return Restore::Miss;
        };
        if identity != current_identity.as_bytes() {
            return Restore::Miss;
        }
        let Some(bitmap) = assets::register_image(cache::lazy_get(IMAGE.name())) else {
            return Restore::Miss;
        };
        STATE.with(|s| {
            *s.borrow_mut() = State::Loaded {
                bitmap,
                aspect: render::aspect_of(w, h),
            };
        });
        let remaining_ms = remaining_ttl_ms(stat.saved_at);
        if remaining_ms > 0 {
            Restore::Fresh { remaining_ms }
        } else {
            Restore::Stale
        }
    }

    // Cache metadata is `[w u32 | h u32 | identity]` (the host write path).
    fn parse_meta(meta: &[u8]) -> Option<(u32, u32, &[u8])> {
        let w = u32::from_le_bytes(meta.get(0..4)?.try_into().ok()?);
        let h = u32::from_le_bytes(meta.get(4..8)?.try_into().ok()?);
        Some((w, h, meta.get(8..)?))
    }

    // Ms left in the refresh interval; 0 once past.
    fn remaining_ttl_ms(saved_at_ms: u64) -> u32 {
        let now_ms = u64::try_from(host::SystemTime::now().unix_secs)
            .unwrap_or(0)
            .saturating_mul(1000);
        let elapsed = now_ms.saturating_sub(saved_at_ms);
        let remaining = u64::from(refresh_interval_ms()).saturating_sub(elapsed);
        u32::try_from(remaining).unwrap_or(u32::MAX)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(delta_ms: u32) {
        // First render restores from cache + schedules the poll here, rather
        // than init(), because renderer imports trap outside a render scope.
        if INITIAL_RESTORE.with(Cell::get) {
            INITIAL_RESTORE.with(|f| f.set(false));
            restore_then_schedule();
        }

        // Advance the menu auto-dismiss countdown.
        if menu_open() {
            let remaining = MENU_MS.with(Cell::get).saturating_sub(delta_ms);
            MENU_MS.with(|m| m.set(remaining));
        }

        let size = widget_size();
        let params = manifest_params::Params::current();
        let base = if params.url.trim().is_empty() {
            render::message_view(render::CONFIGURE_URL, size)
        } else {
            STATE.with(|s| match &*s.borrow() {
                State::Loaded { bitmap, aspect } => {
                    let view = render::image_view(*bitmap, *aspect, size, params.sizing);
                    if REFRESHING.with(Cell::get) {
                        render::with_updating_overlay(view)
                    } else if STALE.with(Cell::get) {
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

        let open = menu_open();
        let result = render_ui(
            size.width,
            size.height,
            render::with_interaction(base, open),
        );

        // Route taps: a button when the menu is open, otherwise open/dismiss it.
        let changed = if open {
            if result.clicks.contains_key(render::KEY_RELOAD) {
                force_reload();
                close_menu();
                true
            } else if result.clicks.contains_key(render::KEY_CLOSE)
                || result.clicks.contains_key(render::KEY_TAP)
            {
                close_menu();
                true
            } else {
                false
            }
        } else if result.clicks.contains_key(render::KEY_TAP) {
            open_menu();
            true
        } else {
            false
        };

        // Re-render on a change; else hold one frame at the dismiss deadline.
        if changed {
            request_frame();
        } else if menu_open() {
            request_frame_after(MENU_MS.with(Cell::get));
        }
    }
}

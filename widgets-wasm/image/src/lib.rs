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

//! Image widget — fetches a picture from an operator-configured URL
//! and displays it fitted to the widget viewport.
//! The picture pipeline itself lives in `remote-image`;
//! this crate owns the URL, the refresh cadence and the poll that drives them.

mod manifest_params;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::manifest_params::{self, Sizing};
    use std::cell::{Cell, RefCell};

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render uses many SDK exports"
    )]
    use bmc_wasm_sdk::*;
    use remote_image::machine::{self, Action, Badge, Event, View};
    use remote_image::{Fit, picture, render};

    /// Menu clears itself this long after opening, untouched.
    const MENU_AUTO_DISMISS_MS: u32 = 10_000;

    const CONFIGURE_URL: &str = "Set an image URL";

    thread_local! {
        static VIEW: RefCell<View> = const { RefCell::new(View::Loading { decode: None }) };
        static POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        // Menu auto-dismiss countdown (ms); 0 = closed.
        static MENU_MS: Cell<u32> = const { Cell::new(0) };
        // First render restores from cache (init() has no renderer scope).
        static INITIAL_RESTORE: Cell<bool> = const { Cell::new(false) };
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let handle = register_poll(
            build_request,
            on_image,
            PollConfig {
                interval_ms: Some(refresh_interval_ms()),
                enabled: false, // first render restores + schedules
                ..Default::default()
            },
        );
        POLL.with(|p| p.set(Some(handle)));
        INITIAL_RESTORE.with(|f| f.set(true));
    }

    /// Fold an event into the view and run its side effects.
    fn dispatch(event: Event) {
        let cur = VIEW.with(|v| v.replace(View::Loading { decode: None }));
        let (next, actions) = machine::step(cur, event);
        VIEW.with(|v| *v.borrow_mut() = next);
        for action in actions {
            match action {
                Action::EnablePollAfter(ms) => with_poll(|h| h.enable_after(ms)),
                Action::ResumePoll => with_poll(|h| {
                    h.set_enabled(true);
                    h.invalidate();
                }),
                Action::DisablePoll => with_poll(|h| h.set_enabled(false)),
                Action::Retry => with_poll(PollHandle::retry),
                Action::DeferPoll => with_poll(|h| h.retry_after(refresh_interval_ms())),
                Action::MarkStale => with_poll(PollHandle::mark_stale),
                Action::SeedAnchor(secs) => with_poll(|h| h.restore_anchor(secs)),
                Action::RequestFrame => request_frame(),
            }
        }
    }

    fn with_poll(f: impl FnOnce(PollHandle)) {
        POLL.with(|p| {
            if let Some(handle) = p.get() {
                f(handle);
            }
        });
    }

    // Zero guard only, deliberately not the manifest's `min`. Staleness fires
    // at `interval * stale_factor`, so clamping a stored interval up here
    // would move that threshold too — quietly weakening the configured freshness.
    fn refresh_interval_ms() -> u32 {
        let secs = manifest_params::Params::current().refresh_seconds.max(1);
        u32::try_from(secs).unwrap_or(u32::MAX).saturating_mul(1000)
    }

    fn fit() -> Fit {
        match manifest_params::Params::current().sizing {
            Sizing::Contain => Fit::Contain,
            Sizing::Cover => Fit::Cover,
        }
    }

    // {{width}}/{{height}} expand to the viewport pixels — 1:1 with the
    // released Slint widget so existing server URLs carry over unchanged.
    fn expanded_url() -> Option<String> {
        let size = widget_size();
        machine::expand_url(
            &manifest_params::Params::current().url,
            size.width,
            size.height,
        )
    }

    // Cache identity: expanded URL + fit, so a URL/viewport/sizing change is a distinct blob.
    fn cache_identity() -> Option<String> {
        let mut id = expanded_url()?;
        id.push('\u{1f}');
        id.push_str(fit().identity_token());
        Some(id)
    }

    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        expanded_url().map(|url| FetchSpec::get(url).host_body())
    }

    fn on_image(_handle: PollHandle, response: &FetchResponse) {
        dispatch(picture::classify_body(
            response,
            widget_size(),
            fit(),
            &cache_identity().unwrap_or_default(),
            on_decoded,
        ));
    }

    fn on_decoded(job: ImageJobId, bitmap: Option<BitmapId>) {
        dispatch(match bitmap {
            Some(bitmap) => Event::Decoded { job, bitmap },
            None => Event::DecodeFailed { job },
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        // Retarget the poll live on a refresh-period change; invalidate to apply now.
        if prev
            .as_ref()
            .is_some_and(|p| p.refresh_seconds != cur.refresh_seconds)
        {
            with_poll(|handle| {
                handle.set_interval(refresh_interval_ms());
                handle.invalidate();
            });
        }
        if prev
            .as_ref()
            .is_none_or(|p| p.url != cur.url || p.sizing != cur.sizing)
        {
            dispatch(Event::TargetChanged);
        } else {
            request_frame();
        }
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
    pub extern "C" fn on_sleep() {
        dispatch(Event::Sleep);
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_wake() {
        INITIAL_RESTORE.with(|f| f.set(false)); // wake subsumes the cold-start restore
        let has_bitmap = VIEW.with(|view| matches!(&*view.borrow(), View::Shown { .. }));
        let identity = cache_identity().unwrap_or_default();
        let interval = refresh_interval_ms();
        dispatch(if has_bitmap {
            picture::wake(&identity, interval)
        } else {
            picture::restore(&identity, interval)
        });
    }

    fn menu_open() -> bool {
        MENU_MS.with(Cell::get) > 0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(delta_ms: u32) {
        // First render restores from cache (init() has no renderer scope).
        if INITIAL_RESTORE.with(Cell::get) {
            INITIAL_RESTORE.with(|f| f.set(false));
            dispatch(picture::restore(
                &cache_identity().unwrap_or_default(),
                refresh_interval_ms(),
            ));
        }

        if menu_open() {
            let remaining = MENU_MS.with(Cell::get).saturating_sub(delta_ms);
            MENU_MS.with(|m| m.set(remaining));
        }

        let size = widget_size();
        let params = manifest_params::Params::current();
        let base = if params.url.trim().is_empty() {
            render::message_view(CONFIGURE_URL, size)
        } else {
            VIEW.with(|v| match &*v.borrow() {
                View::Shown {
                    bitmap,
                    aspect,
                    badge,
                    ..
                } => {
                    let view = render::image_view(*bitmap, *aspect, size, fit());
                    match badge {
                        Badge::Updating => {
                            with_overlay(view, render::updating_pill(), widget_viewport().shape)
                        }
                        // is_stale adds the grace window, so the 10s retry heals a blip first.
                        Badge::Stale => match POLL
                            .with(Cell::get)
                            .filter(|handle| handle.is_stale())
                            .and_then(PollHandle::last_success_time)
                        {
                            Some(anchor) => {
                                with_stale_overlay(view, anchor, widget_viewport().shape)
                            }
                            None => view,
                        },
                        // A broken payload states its specific reason at once.
                        Badge::Error(kind) => with_error_overlay(
                            view,
                            render::error_message(*kind),
                            widget_viewport().shape,
                        ),
                        Badge::Fresh => view,
                    }
                }
                View::Loading { .. } => render::message_view(render::LOADING, size),
                View::Failed(kind) => render::message_view(render::error_message(*kind), size),
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
                dispatch(Event::Reload);
                MENU_MS.with(|m| m.set(0));
                true
            } else if result.clicks.contains_key(render::KEY_CLOSE)
                || result.clicks.contains_key(render::KEY_TAP)
            {
                MENU_MS.with(|m| m.set(0));
                true
            } else {
                false
            }
        } else if result.clicks.contains_key(render::KEY_TAP) {
            MENU_MS.with(|m| m.set(MENU_AUTO_DISMISS_MS));
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

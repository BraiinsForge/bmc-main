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

//! Picture of the Day — shows the current NASA Astronomy Picture of the Day
//! with its title and credit, served by nexus.
//!
//! Two polls. The feed poll asks `latest_full.json` every half hour
//! and costs about a kilobyte.
//! The picture poll fires only when that metadata names a date
//! the widget is not already showing, so a picture arrives once a day at most.
//! Neither depends on the device clock being right:
//! the feed names the date, the widget only compares it against the one on flash.
//!
//! The picture pipeline itself (decode, flash cache, restore, dormancy) is
//! `remote-image`, shared with the Image widget.

mod caption;
mod feed;
mod manifest_params;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::caption;
    use super::feed::{self, Meta};
    use super::manifest_params;
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render uses many SDK exports"
    )]
    use bmc_wasm_sdk::*;
    use remote_image::machine::{self, Action, Badge, Event, View};
    use remote_image::{Fit, picture, render};

    /// How often the feed is asked whether a new picture has been published.
    const FEED_INTERVAL_MS: u32 = 30 * 60 * 1000;
    /// Backoff after an unreachable or unusable feed reply.
    /// Nothing else is trying until the feed answers once, so this alone
    /// decides how long a Deck that booted before its network sits empty.
    /// A kilobyte of JSON that nexus serves from cache is cheap at this cadence.
    const FEED_RETRY_MS: u32 = 30 * 1000;
    /// Backoff after an unreachable or truncated picture body.
    /// Only reachable once the feed has answered, so it never delays first load,
    /// and megabytes are worth waiting out a blip for.
    const PICTURE_RETRY_MS: u32 = 5 * 60 * 1000;
    /// Backoff after a body that transferred in full and would not decode.
    /// That is no blip, so it waits out a feed cycle rather than the picture retry.
    /// Nothing longer is observable: the feed re-arms the picture poll
    /// on every check that still names a date the widget is not showing.
    const DEFER_MS: u32 = FEED_INTERVAL_MS;
    /// Menu clears itself this long after opening, untouched.
    const MENU_AUTO_DISMISS_MS: u32 = 10_000;

    /// The feed decides when a cached picture is out of date, not a clock,
    /// so hand the pipeline the longest time-to-live it can carry.
    /// That is 49.7 days rather than forever, which costs one re-download
    /// on a Deck that has sat on the same picture for seven weeks.
    const PICTURE_TTL_MS: u32 = u32::MAX;

    /// A new picture is published once a day.
    const PICTURE_PUBLISH_SECS: u64 = 24 * 60 * 60;
    /// How old the shown picture must be, with the fetch still failing,
    /// before the overlay calls it stale.
    /// The picture poll carries no interval for `is_stale` to measure against,
    /// so the cadence above stands in for one at the SDK's own `stale_factor`.
    const PICTURE_STALE_AFTER: Duration = Duration::from_secs(PICTURE_PUBLISH_SECS * 3 / 2);

    /// Flash cache tag for the caption. Distinct from the picture's own tag,
    /// and reclaimed with it when the instance goes away.
    const CAPTION_TAG: &str = "caption";

    thread_local! {
        static VIEW: RefCell<View> = const { RefCell::new(View::Loading { decode: None }) };
        /// What the feed last said. Drives what to fetch, never what to draw.
        static PENDING: RefCell<Meta> = const { RefCell::new(Meta::new()) };
        /// The displayed picture's cache identity and the caption that belongs
        /// to it. Set together so a credit can never be drawn over a picture it
        /// does not describe.
        static SHOWN: RefCell<Option<(String, Meta)>> = const { RefCell::new(None) };
        /// What the in-flight picture request was built for.
        /// The reply carries no URL, and a feed poll can land between the two,
        /// so recomputing the identity at reply time would file the body
        /// under the wrong date.
        static REQUESTED: RefCell<Option<(String, Meta)>> = const { RefCell::new(None) };
        /// The same pair for the outstanding decode, keyed by its job,
        /// so a superseded completion cannot install its caption.
        static DECODING: RefCell<Option<(ImageJobId, String, Meta)>> = const { RefCell::new(None) };
        static FEED_POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        static PICTURE_POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        // Menu auto-dismiss countdown (ms); 0 = closed.
        static MENU_MS: Cell<u32> = const { Cell::new(0) };
        // First render restores from cache (init() has no renderer scope).
        static INITIAL_RESTORE: Cell<bool> = const { Cell::new(false) };
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let feed_poll = register_poll(
            build_feed_request,
            on_feed,
            PollConfig {
                interval_ms: Some(FEED_INTERVAL_MS),
                retry_ms: FEED_RETRY_MS,
                ..Default::default()
            },
        );
        FEED_POLL.with(|p| p.set(Some(feed_poll)));

        // No interval: the feed arms this one, and a success must not
        // reschedule a re-download of a picture that has not changed.
        let picture_poll = register_poll(
            build_picture_request,
            on_picture,
            PollConfig {
                interval_ms: None,
                retry_ms: PICTURE_RETRY_MS,
                enabled: false,
                ..Default::default()
            },
        );
        PICTURE_POLL.with(|p| p.set(Some(picture_poll)));

        INITIAL_RESTORE.with(|f| f.set(true));
    }

    // ── State machine plumbing ───────────────────────────────────────

    /// Fold an event into the view and run its side effects.
    fn dispatch(event: Event) {
        let cur = VIEW.with(|v| v.replace(View::Loading { decode: None }));
        let (next, actions) = machine::step(cur, event);
        VIEW.with(|v| *v.borrow_mut() = next);
        for action in actions {
            match action {
                // The picture has no time-to-live to schedule against; the feed
                // decides when a new one exists.
                Action::EnablePollAfter(_) => {}
                Action::ResumePoll => with_picture_poll(|h| {
                    h.set_enabled(true);
                    h.invalidate();
                }),
                Action::DisablePoll => with_picture_poll(|h| h.set_enabled(false)),
                Action::Retry => with_picture_poll(PollHandle::retry),
                Action::DeferPoll => with_picture_poll(|h| h.retry_after(DEFER_MS)),
                // Both anchor the picture, not the feed: a late decode failure
                // invalidates the picture fetch that just banked,
                // and a restored timestamp is when that picture reached flash.
                Action::MarkStale => with_picture_poll(PollHandle::mark_stale),
                Action::SeedAnchor(secs) => with_picture_poll(|h| h.restore_anchor(secs)),
                Action::RequestFrame => request_frame(),
            }
        }
    }

    fn with_feed_poll(f: impl FnOnce(PollHandle)) {
        FEED_POLL.with(|p| {
            if let Some(handle) = p.get() {
                f(handle);
            }
        });
    }

    fn with_picture_poll(f: impl FnOnce(PollHandle)) {
        PICTURE_POLL.with(|p| {
            if let Some(handle) = p.get() {
                f(handle);
            }
        });
    }

    // ── Target ───────────────────────────────────────────────────────

    /// nexus contains the picture to the box we ask for, so it arrives already
    /// fitted and the host only letterboxes it. There is no sizing to choose.
    const FIT: Fit = Fit::Contain;

    fn source() -> manifest_params::Source {
        manifest_params::Params::current().source
    }

    /// The identity the widget should be showing, or `None` while the feed has
    /// not yet named a date.
    fn target_identity() -> Option<String> {
        let size = widget_size();
        PENDING.with(|p| {
            let date = &p.borrow().date;
            (!date.is_empty()).then(|| feed::picture_url(source(), date, size.width, size.height))
        })
    }

    fn shown_identity() -> Option<String> {
        SHOWN.with(|s| s.borrow().as_ref().map(|(id, _)| id.clone()))
    }

    // ── Feed poll ────────────────────────────────────────────────────

    #[expect(
        clippy::unnecessary_wraps,
        reason = "matches the SDK Build callback signature; the feed URL is fixed"
    )]
    fn build_feed_request(_handle: PollHandle) -> Option<FetchSpec> {
        Some(FetchSpec::get(feed::metadata_url(source())))
    }

    fn on_feed(handle: PollHandle, response: &FetchResponse) {
        if !response.ok() {
            // The poll engine reschedules on retry_ms.
            log_warn!(
                "picture-of-the-day: feed fetch failed (status {})",
                response.status
            );
            return;
        }
        let doc = response.json();
        let date = doc.str("/data/date").unwrap_or_default();
        if !feed::is_published_date(&date) {
            // A 2xx that names no usable date is nexus misbehaving,
            // not a bad configuration, so retry rather than parking for the interval.
            log_warn!("picture-of-the-day: feed /data/date is not a published date");
            handle.retry();
            return;
        }
        let meta = Meta {
            date,
            title: feed::one_line(&doc.str("/data/title").unwrap_or_default()),
            credit: feed::one_line(&doc.str("/data/copyright").unwrap_or_default()),
        };
        PENDING.with(|p| *p.borrow_mut() = meta.clone());

        if target_identity() == shown_identity() {
            // Same picture. Take any corrected wording, and leave the poll alone.
            let rewritten = SHOWN.with(|s| match &mut *s.borrow_mut() {
                Some((_, shown)) if *shown != meta => {
                    *shown = meta.clone();
                    true
                }
                _ => false,
            });
            if rewritten {
                write_caption(&meta);
                request_frame();
            }
        } else {
            // Superseded, not wrong: a picture already on screen keeps its place
            // until the new one decodes, so a publication we cannot fetch
            // does not cost the reader the one they already had.
            dispatch(Event::TargetSuperseded);
        }
    }

    // ── Picture poll ─────────────────────────────────────────────────

    fn build_picture_request(_handle: PollHandle) -> Option<FetchSpec> {
        let size = widget_size();
        let meta = PENDING.with(|p| p.borrow().clone());
        if meta.date.is_empty() {
            return None;
        }
        let identity = feed::picture_url(source(), &meta.date, size.width, size.height);
        let spec = FetchSpec::get(identity.clone()).host_body();
        REQUESTED.with(|r| *r.borrow_mut() = Some((identity, meta)));
        Some(spec)
    }

    fn on_picture(_handle: PollHandle, response: &FetchResponse) {
        let Some((identity, meta)) = REQUESTED.with(|r| r.borrow().clone()) else {
            return;
        };
        let event = picture::classify_body(response, widget_size(), FIT, &identity, on_decoded);
        if let Event::DecodeStarted { job, .. } = event {
            DECODING.with(|d| *d.borrow_mut() = Some((job, identity, meta)));
        }
        dispatch(event);
    }

    fn on_decoded(job: ImageJobId, bitmap: Option<BitmapId>) {
        let decoded = DECODING.with(|d| {
            let mut slot = d.borrow_mut();
            let owns_job = matches!(&*slot, Some((pending, ..)) if *pending == job);
            if owns_job { slot.take() } else { None }
        });
        if bitmap.is_some()
            && let Some((_, identity, meta)) = decoded
        {
            debug_assert!(
                feed::is_from(source(), &identity),
                "BUG: captioning a picture the current feed did not serve"
            );
            write_caption(&meta);
            SHOWN.with(|s| *s.borrow_mut() = Some((identity, meta)));
        }
        dispatch(match bitmap {
            Some(bitmap) => Event::Decoded { job, bitmap },
            None => Event::DecodeFailed { job },
        });
    }

    // ── Caption persistence ──────────────────────────────────────────

    fn write_caption(meta: &Meta) {
        cache::put(CAPTION_TAG, &[], meta.encode().as_bytes());
    }

    fn read_caption() -> Option<Meta> {
        let entry = cache::read_bytes(CAPTION_TAG)?;
        Meta::decode(core::str::from_utf8(&entry.bytes).ok()?)
    }

    // ── Lifecycle ────────────────────────────────────────────────────

    /// Bring back whichever picture is on flash, without waiting for the feed.
    ///
    /// The caption is adopted only when it describes the very blob being
    /// restored — the two cache entries are written separately and a crash
    /// between them would otherwise caption someone else's photograph.
    /// Render scope only.
    fn restore_from_cache() -> Event {
        let size = widget_size();
        let Some(stored) = picture::stored_identity() else {
            return Event::RestoreMiss;
        };
        // `restore` matches the identity against the metadata it was just read
        // from, so it never refuses. A source change delivered off-screen would
        // therefore put the old feed's picture back up on the next wake, where
        // the same change on-screen clears the face.
        if !feed::is_from(source(), &stored) {
            return Event::RestoreMiss;
        }
        let event = picture::restore(&stored, PICTURE_TTL_MS);
        if matches!(event, Event::RestoreMiss) {
            return event;
        }
        let fits = |meta: Meta| {
            meta.describes(source(), &stored, size.width, size.height)
                .then_some(meta)
        };
        let meta = match read_caption().and_then(&fits) {
            Some(meta) => Some(meta),
            // A decode that lands while dormant updates the picture entry, not
            // the caption. The feed's last answer still names that picture,
            // so the credit skips a round trip and flash catches up here.
            None => PENDING
                .with(|p| fits(p.borrow().clone()))
                .inspect(write_caption),
        };
        if let Some(meta) = meta {
            // Only while the feed has said nothing: its answer outranks flash,
            // and a cold-boot reply can beat the first render,
            // so overwriting it would caption the in-flight picture with this older one.
            PENDING.with(|p| {
                let mut pending = p.borrow_mut();
                if pending.date.is_empty() {
                    *pending = meta.clone();
                }
            });
            SHOWN.with(|s| *s.borrow_mut() = Some((stored, meta)));
        } else {
            SHOWN.with(|s| *s.borrow_mut() = Some((stored, Meta::new())));
        }
        event
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        // A different feed is a different picture; the title toggle only
        // changes what is drawn over the one already held.
        if prev.as_ref().is_none_or(|p| p.source != cur.source) {
            // The date the old feed named means nothing in the new feed's URL
            // space, and pairing the two would fetch one feed's picture under
            // the other's title. Dropping it leaves the picture poll on the
            // empty-date guard in `build_picture_request` until the new feed
            // has answered, so ask it now rather than at the end of its
            // half hour. Inert while dormant; `on_wake` asks again.
            PENDING.with(|p| *p.borrow_mut() = Meta::new());
            with_feed_poll(PollHandle::invalidate);
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
        with_feed_poll(|h| h.set_enabled(false));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_wake() {
        INITIAL_RESTORE.with(|f| f.set(false)); // wake subsumes the cold-start restore
        // Off-scene time is unbounded, so ask the feed straight away rather
        // than waiting out the remainder of its interval.
        with_feed_poll(|h| {
            h.set_enabled(true);
            h.invalidate();
        });
        dispatch(restore_from_cache());
    }

    fn menu_open() -> bool {
        MENU_MS.with(Cell::get) > 0
    }

    // ── Render ───────────────────────────────────────────────────────

    /// The picture with its caption, wearing whatever badge the view carries.
    fn picture_face(bitmap: BitmapId, aspect: f32, badge: Badge, size: WidgetSize) -> Node {
        let show_title = manifest_params::Params::current().show_title;
        let view = SHOWN.with(|s| {
            let shown = s.borrow();
            let meta = shown.as_ref().map(|(_, meta)| meta);
            caption::with_caption(
                render::image_view(bitmap, aspect, size, FIT),
                meta.filter(|_| show_title).map(|m| m.title.as_str()),
                meta.map_or("", |m| m.credit.as_str()),
                size,
            )
        });
        let shape = widget_viewport().shape;
        match badge {
            Badge::Updating => with_overlay(view, render::updating_pill(), shape),
            // The badge already means the last picture fetch failed,
            // so only the grace window is left to check.
            Badge::Stale => match PICTURE_POLL
                .with(Cell::get)
                .filter(|handle| {
                    handle
                        .last_success_age()
                        .is_some_and(|age| age > PICTURE_STALE_AFTER)
                })
                .and_then(PollHandle::last_success_time)
            {
                Some(anchor) => with_stale_overlay(view, anchor, shape),
                None => view,
            },
            // A broken payload states its specific reason at once.
            Badge::Error(kind) => with_error_overlay(view, render::error_message(kind), shape),
            Badge::Fresh => view,
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(delta_ms: u32) {
        // First render restores from cache (init() has no renderer scope).
        if INITIAL_RESTORE.with(Cell::get) {
            INITIAL_RESTORE.with(|f| f.set(false));
            dispatch(restore_from_cache());
        }

        if menu_open() {
            let remaining = MENU_MS.with(Cell::get).saturating_sub(delta_ms);
            MENU_MS.with(|m| m.set(remaining));
        }

        let size = widget_size();
        let base = VIEW.with(|v| match &*v.borrow() {
            View::Shown {
                bitmap,
                aspect,
                badge,
                ..
            } => picture_face(*bitmap, *aspect, *badge, size),
            View::Loading { .. } => render::message_view(render::LOADING, size),
            View::Failed(kind) => render::message_view(render::error_message(*kind), size),
        });

        let open = menu_open();
        let result = render_ui(
            size.width,
            size.height,
            render::with_interaction(base, open),
        );

        // Route taps: a button when the menu is open, otherwise open/dismiss it.
        let changed = if open {
            if result.clicks.contains_key(render::KEY_RELOAD) {
                // Re-ask the feed too: the picture is only stale if the date moved.
                with_feed_poll(|h| {
                    h.set_enabled(true);
                    h.invalidate();
                });
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

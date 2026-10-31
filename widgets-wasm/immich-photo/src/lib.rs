// Copyright (C) 2026  Braiins Forge s.r.o.
// Copyright (C) 2026  Jan Novák
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

//! Immich photo widget — random photos straight from an Immich server, with a
//! **Label it** menu button that tags the photo on screen in Immich and turns
//! into **Remove label** once the photo has that tag.
//!
//! The picture pipeline is the image widget's. The Immich API key lives in a
//! credential slot: requests carry its placeholder and the device fills it in.
//! Each refresh shows the next id of a small queue that `POST
//! /api/search/random` keeps filled; labelling upserts the tag and adds or
//! removes the photo.

mod manifest_params;

/// Random photos asked for at a time; the queue refills when it runs low.
pub const QUEUE_BATCH: usize = 10;
/// Immich's small thumbnail edge; larger widgets get the preview.
pub const THUMBNAIL_PX: u32 = 250;

/// `server` joined with an API path, without doubling the slash.
#[must_use]
pub fn api_url(server: &str, path: &str) -> String {
    format!("{}{}", server.trim_end_matches('/'), path)
}

/// The Immich thumbnail size that suits a widget of `width` × `height`.
#[must_use]
pub fn thumbnail_size(width: u32, height: u32) -> &'static str {
    if width.max(height) <= THUMBNAIL_PX {
        "thumbnail"
    } else {
        "preview"
    }
}

/// Body for `POST /api/search/random`: timeline photos only, so videos,
/// archived and hidden assets stay out.
#[must_use]
pub fn random_body() -> String {
    format!("{{\"size\":{QUEUE_BATCH},\"type\":\"IMAGE\",\"visibility\":\"timeline\"}}")
}

/// Body for `PUT /api/tags`, which creates the tag or returns the existing one.
#[must_use]
pub fn tag_body(label: &str) -> String {
    format!("{{\"tags\":[\"{}\"]}}", json_escape(label))
}

/// Body for adding or removing one asset at `/api/tags/{id}/assets`.
#[must_use]
pub fn ids_body(asset: &str) -> String {
    format!("{{\"ids\":[\"{}\"]}}", json_escape(asset))
}

/// Whether an Immich bulk-id result counts as done: a photo that already
/// has the tag, or no longer has it, needs nothing more.
#[must_use]
pub fn bulk_done(success: Option<bool>, error: Option<&str>) -> bool {
    success == Some(true) || matches!(error, Some("duplicate" | "not_found"))
}

fn json_escape(s: &str) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// A param counts as set only with something besides whitespace in it.
#[must_use]
pub fn configured(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_url_joins_with_one_slash() {
        assert_eq!(
            api_url("http://h:2283", "/api/tags"),
            "http://h:2283/api/tags"
        );
        assert_eq!(api_url("https://h/", "/api/tags"), "https://h/api/tags");
    }

    #[test]
    fn thumbnail_size_follows_the_widget() {
        assert_eq!(thumbnail_size(250, 200), "thumbnail");
        assert_eq!(thumbnail_size(317, 238), "preview");
    }

    #[test]
    fn bodies_are_json() {
        assert_eq!(
            random_body(),
            r#"{"size":10,"type":"IMAGE","visibility":"timeline"}"#
        );
        assert_eq!(tag_body("a \"b\" \\ c"), r#"{"tags":["a \"b\" \\ c"]}"#);
        assert_eq!(tag_body("Dovolená\n"), r#"{"tags":["Dovolená\n"]}"#);
        assert_eq!(ids_body("x\u{1}"), r#"{"ids":["x\u0001"]}"#);
    }

    #[test]
    fn bulk_done_accepts_no_ops() {
        assert!(bulk_done(Some(true), None));
        assert!(bulk_done(Some(false), Some("duplicate")));
        assert!(bulk_done(Some(false), Some("not_found")));
        assert!(!bulk_done(Some(false), Some("no_permission")));
        assert!(!bulk_done(None, None));
    }

    #[test]
    fn configured_ignores_blank_values() {
        assert_eq!(configured(None), None);
        assert_eq!(configured(Some("  ")), None);
        assert_eq!(configured(Some(" Print ")), Some("Print"));
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::manifest_params::{self, Sizing, credentials};
    use super::{api_url, bulk_done, configured, ids_body, random_body, tag_body, thumbnail_size};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    use bmc_wasm_sdk::json::JsonDoc;
    use bmc_wasm_sdk::net::FetchRequest;
    use bmc_wasm_sdk::*;
    use remote_image::machine::{self, Action, Badge, Event, View};
    use remote_image::{Fit, picture, render};

    /// Menu clears itself this long after opening, untouched.
    const MENU_AUTO_DISMISS_MS: u32 = 10_000;
    /// How long a label change's outcome stays on screen.
    const NOTICE_MS: u32 = 3_000;
    /// Wait before asking Immich again after the photo list failed.
    const REFILL_RETRY_MS: u32 = 30_000;
    /// Refill once the queue is down to this many ids.
    const QUEUE_LOW: usize = 2;

    const CONFIGURE_SERVER: &str = "Set the Immich address";
    const CHECK_SETUP: &str = "Check the Immich address and API key";
    const NO_PHOTOS: &str = "No photos in Immich";
    const KEY_LABEL: &str = "menu_label";

    const ICON_RENEW: Svg = include_svg!("assets/renew.svg");
    const MENU_WIDTH: f32 = 200.0;
    const MENU_GAP: f32 = 8.0;
    const MENU_PADDING: f32 = 12.0;
    const NOTICE_TEXT_SIZE: u32 = 14;

    /// A label change in progress: one or two Immich requests.
    #[derive(Clone)]
    struct Pending {
        asset: String,
        label: String,
        remove: bool,
    }

    thread_local! {
        static VIEW: RefCell<View> = const { RefCell::new(View::Loading { decode: None }) };
        static POLL: RefCell<Option<PollHandle>> = const { RefCell::new(None) };
        // Menu auto-dismiss countdown (ms); 0 = closed.
        static MENU_MS: Cell<u32> = const { Cell::new(0) };
        // First render restores from cache (init() has no renderer scope).
        static INITIAL_RESTORE: Cell<bool> = const { Cell::new(false) };
        /// Off-scene, between `on_sleep` and `on_wake`. The host keeps
        /// delivering params and fetch replies there, and no SDK call answers
        /// it, so the edge is tracked by hand. It starts set: every slot is
        /// born dormant and `on_wake` always precedes the first frame, so a
        /// widget built for a scene nobody opens must not fetch a picture.
        static DORMANT: Cell<bool> = const { Cell::new(true) };
        /// Random asset ids still to show, and whether more are on the way.
        static QUEUE: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
        static REFILLING: Cell<bool> = const { Cell::new(false) };
        /// The poll found the queue empty and waits for the refill.
        static STARVED: Cell<bool> = const { Cell::new(false) };
        /// Asset the photo fetch in flight asked for.
        static REQUESTED: RefCell<Option<String>> = const { RefCell::new(None) };
        /// Immich asset of the photo on screen. A photo restored from flash
        /// after a restart has none until the next fetch.
        static ASSET: RefCell<Option<String>> = const { RefCell::new(None) };
        /// Immich tags of that asset, as fetched and as changed since.
        static TAGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        /// Why no photo comes, shown while there's nothing else to show.
        static PROBLEM: Cell<Option<&'static str>> = const { Cell::new(None) };
        /// The label change on its way; further taps wait for it.
        static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
        /// Label change outcome on screen, and how long it stays (ms).
        static NOTICE: RefCell<Option<String>> = const { RefCell::new(None) };
        static NOTICE_LEFT_MS: Cell<u32> = const { Cell::new(0) };
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
        POLL.set(Some(handle));
        INITIAL_RESTORE.with(|f| f.set(true));
    }

    /// Fold an event into the view and run its side effects.
    fn dispatch(event: Event) {
        let cur = VIEW.with(|v| v.replace(View::Loading { decode: None }));
        let (next, actions) = machine::step(cur, event);
        VIEW.with(|v| *v.borrow_mut() = next);
        let dormant = DORMANT.with(Cell::get);
        for action in actions {
            if dormant && action.starts_fetch() {
                continue;
            }
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

    fn with_poll(f: impl FnOnce(&PollHandle)) {
        POLL.with_borrow(|p| {
            if let Some(handle) = p {
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

    fn server() -> Option<String> {
        configured(manifest_params::Params::current().server.as_deref()).map(str::to_owned)
    }

    fn label() -> Option<String> {
        configured(manifest_params::Params::current().label.as_deref()).map(str::to_owned)
    }

    fn key_header() -> String {
        fmt!("x-api-key: {}", credentials::immich::TOKEN)
    }

    fn json_headers() -> String {
        fmt!(
            "x-api-key: {}\nContent-Type: application/json",
            credentials::immich::TOKEN
        )
    }

    // Cache identity: server + viewport + fit, so a change of any is a distinct blob.
    fn cache_identity() -> Option<String> {
        let size = widget_size();
        Some(fmt!(
            "{}\u{1f}{}x{}\u{1f}{}",
            server()?,
            size.width,
            size.height,
            fit().identity_token()
        ))
    }

    /// Ask Immich for the next batch of random photos, unless already asking.
    fn refill() {
        let Some(server) = server() else {
            return;
        };
        if REFILLING.with(Cell::get) {
            return;
        }
        let url = api_url(&server, "/api/search/random");
        let body = random_body();
        let sent = FetchRequest::post(&url)
            .headers(&json_headers())
            .body(body.as_bytes())
            .send(on_refilled);
        REFILLING.with(|r| r.set(sent.is_some()));
    }

    fn on_refilled(response: &FetchResponse) {
        REFILLING.with(|r| r.set(false));
        let ids: Option<Vec<String>> = response.ok().then(|| {
            let doc = JsonDoc::parse(response.body());
            (0..doc.len("").unwrap_or(0))
                .filter_map(|i| doc.str(&fmt!("/{}/id", i)))
                .collect()
        });
        match ids {
            Some(ids) if !ids.is_empty() => {
                PROBLEM.with(|p| p.set(None));
                QUEUE.with_borrow_mut(|q| q.extend(ids));
                // The poll skipped its turn for want of an id; take it now.
                if STARVED.with(|s| s.replace(false)) {
                    with_poll(PollHandle::invalidate);
                }
            }
            Some(_) => PROBLEM.with(|p| p.set(Some(NO_PHOTOS))),
            None => {
                log_warn!("immich-photo: photo list refused or failed");
                PROBLEM.with(|p| p.set(Some(CHECK_SETUP)));
                if STARVED.with(|s| s.replace(false)) {
                    with_poll(|h| h.retry_after(REFILL_RETRY_MS));
                }
            }
        }
        request_frame();
    }

    fn build_request(_handle: &PollHandle) -> Option<FetchSpec> {
        let server = server()?;
        let next = QUEUE.with_borrow_mut(VecDeque::pop_front);
        if QUEUE.with_borrow(VecDeque::len) <= QUEUE_LOW {
            refill();
        }
        let Some(asset) = next else {
            STARVED.with(|s| s.set(true));
            return None;
        };
        let size = widget_size();
        let url = api_url(
            &server,
            &fmt!(
                "/api/assets/{}/thumbnail?size={}",
                asset,
                thumbnail_size(size.width, size.height)
            ),
        );
        REQUESTED.set(Some(asset));
        Some(FetchSpec::get(url).headers(key_header()).host_body())
    }

    /// Bring back whichever picture is on flash. Render scope only.
    fn restore_from_cache() -> Event {
        picture::restore(&cache_identity().unwrap_or_default(), refresh_interval_ms())
    }

    fn on_image(_handle: &PollHandle, response: &FetchResponse) {
        let requested = REQUESTED.take();
        if response.ok() {
            ASSET.set(requested.clone());
            TAGS.set(Vec::new());
            if let Some(asset) = requested {
                fetch_tags(&asset);
            }
        }
        dispatch(picture::classify_body(
            response,
            widget_size(),
            fit(),
            &cache_identity().unwrap_or_default(),
            VIEW.with(|v| v.borrow().decode()),
            on_decoded,
        ));
    }

    fn on_decoded(job: ImageJobId, bitmap: Option<BitmapId>) {
        dispatch(match bitmap {
            Some(bitmap) => Event::Decoded { job, bitmap },
            None => Event::DecodeFailed { job },
        });
    }

    /// Learn the photo's tags, so the menu can offer Remove label.
    fn fetch_tags(asset: &str) {
        let Some(server) = server() else {
            return;
        };
        let url = api_url(&server, &fmt!("/api/assets/{}", asset));
        let _ = FetchRequest::get(&url)
            .headers(&key_header())
            .send(on_asset_info);
    }

    fn on_asset_info(response: &FetchResponse) {
        if !response.ok() {
            return;
        }
        let doc = JsonDoc::parse(response.body());
        // Only the photo still on screen.
        if doc.str("/id") != ASSET.with_borrow(Clone::clone) {
            return;
        }
        let tags = (0..doc.len("/tags").unwrap_or(0))
            .filter_map(|i| doc.str(&fmt!("/tags/{}/value", i)))
            .collect();
        TAGS.set(tags);
        request_frame();
    }

    /// Whether the photo on screen carries the configured label.
    fn labelled() -> bool {
        label().is_some_and(|label| {
            ASSET.with_borrow(Option::is_some) && TAGS.with_borrow(|tags| tags.contains(&label))
        })
    }

    fn show_notice(message: String) {
        NOTICE.set(Some(message));
        NOTICE_LEFT_MS.with(|n| n.set(NOTICE_MS));
        request_frame();
    }

    fn failed_notice(remove: bool) -> &'static str {
        if remove {
            "Remove label failed"
        } else {
            "Label it failed"
        }
    }

    /// Start a label change: upsert the tag and add the photo, or look the
    /// tag up and take the photo out.
    fn send_label() {
        let (Some(label), Some(server)) = (label(), server()) else {
            return;
        };
        if PENDING.with_borrow(Option::is_some) {
            return;
        }
        let Some(asset) = ASSET.with_borrow(Clone::clone) else {
            show_notice("Reload the photo first".to_owned());
            return;
        };
        let remove = labelled();
        let url = api_url(&server, "/api/tags");
        let sent = if remove {
            FetchRequest::get(&url)
                .headers(&key_header())
                .send(on_tags_listed)
        } else {
            let body = tag_body(&label);
            FetchRequest::put(&url)
                .headers(&json_headers())
                .body(body.as_bytes())
                .send(on_tag_upserted)
        };
        if sent.is_none() {
            show_notice(failed_notice(remove).to_owned());
            return;
        }
        show_notice(if remove {
            fmt!("Removing {}…", label)
        } else {
            fmt!("Labelling as {}…", label)
        });
        PENDING.set(Some(Pending {
            asset,
            label,
            remove,
        }));
    }

    /// Add or remove the pending photo at the tag `tag_id`.
    fn change_tag_assets(tag_id: &str) {
        let (Some(pending), Some(server)) = (PENDING.with_borrow(Clone::clone), server()) else {
            finish(false);
            return;
        };
        let url = api_url(&server, &fmt!("/api/tags/{}/assets", tag_id));
        let body = ids_body(&pending.asset);
        let headers = json_headers();
        let request = if pending.remove {
            FetchRequest::delete(&url)
        } else {
            FetchRequest::put(&url)
        };
        if request
            .headers(&headers)
            .body(body.as_bytes())
            .send(on_tag_changed)
            .is_none()
        {
            finish(false);
        }
    }

    fn on_tag_upserted(response: &FetchResponse) {
        match response
            .ok()
            .then(|| JsonDoc::parse(response.body()).str("/0/id"))
            .flatten()
        {
            Some(tag_id) => change_tag_assets(&tag_id),
            None => finish(false),
        }
    }

    fn on_tags_listed(response: &FetchResponse) {
        if !response.ok() {
            finish(false);
            return;
        }
        let Some(label) = PENDING.with_borrow(|p| p.as_ref().map(|p| p.label.clone())) else {
            return;
        };
        let doc = JsonDoc::parse(response.body());
        let tag_id = (0..doc.len("").unwrap_or(0))
            .find(|i| doc.str(&fmt!("/{}/value", i)).as_deref() == Some(label.as_str()))
            .and_then(|i| doc.str(&fmt!("/{}/id", i)));
        match tag_id {
            Some(tag_id) => change_tag_assets(&tag_id),
            // No such tag: the photo doesn't have it, nothing to remove.
            None => finish(true),
        }
    }

    fn on_tag_changed(response: &FetchResponse) {
        let done = response.ok() && {
            let doc = JsonDoc::parse(response.body());
            bulk_done(doc.bool("/0/success"), doc.str("/0/error").as_deref())
        };
        finish(done);
    }

    fn finish(ok: bool) {
        let Some(Pending {
            asset,
            label,
            remove,
        }) = PENDING.take()
        else {
            return;
        };
        if !ok {
            log_warn!("immich-photo: label change on {} failed", asset);
            show_notice(failed_notice(remove).to_owned());
            return;
        }
        // The photo may have moved on meanwhile; only its own tags change.
        if ASSET.with_borrow(|a| a.as_deref() == Some(asset.as_str())) {
            TAGS.with_borrow_mut(|tags| {
                tags.retain(|t| *t != label);
                if !remove {
                    tags.push(label.clone());
                }
            });
        }
        show_notice(if remove {
            fmt!("Removed {}", label)
        } else {
            fmt!("Labelled {}", label)
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        // Retarget the poll live on a refresh-period change; invalidate to apply
        // now. Both are safe off-screen: the interval is only stored, and
        // `invalidate` skips a poll `on_sleep` disabled.
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
            .is_none_or(|p| p.server != cur.server || p.sizing != cur.sizing)
        {
            // A new server: forget the old one's photos.
            if prev.as_ref().is_some_and(|p| p.server != cur.server) {
                QUEUE.with_borrow_mut(VecDeque::clear);
                ASSET.set(None);
                TAGS.set(Vec::new());
                PROBLEM.with(|p| p.set(None));
            }
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
        DORMANT.with(|d| d.set(true));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_wake() {
        DORMANT.with(|d| d.set(false));
        INITIAL_RESTORE.with(|f| f.set(false)); // wake subsumes the cold-start restore
        if let Some(job) = VIEW.with(|v| picture::abandoned_decode(&v.borrow())) {
            dispatch(Event::DecodeAbandoned { job });
        }
        dispatch(restore_from_cache());
    }

    fn menu_open() -> bool {
        MENU_MS.with(Cell::get) > 0
    }

    /// Full-bleed transparent catcher (absolute via insets): opens / tap-outside-dismisses.
    fn tap_catcher() -> Node {
        touchable(
            render::KEY_TAP,
            props!(inset_top: 0.0, inset_right: 0.0, inset_bottom: 0.0, inset_left: 0.0),
            Vec::<Draw>::new(),
        )
    }

    /// The reveal menu: Reload, the label button (only when a label is set up)
    /// and Close. The label button reads Remove label once the photo has it.
    fn menu_panel(label_button: Option<&'static str>) -> Node {
        let mut buttons = vec![
            button!(render::KEY_RELOAD, "Reload", style: Primary, icon: ensure_registered(&ICON_RENEW)).stretch(),
        ];
        if let Some(caption) = label_button {
            buttons.push(button!(KEY_LABEL, caption, style: Secondary).stretch());
        }
        buttons.push(button!(render::KEY_CLOSE, "Close", style: Secondary).stretch());
        center(
            props!(inset_top: 0.0, inset_right: 0.0, inset_bottom: 0.0, inset_left: 0.0),
            [col(
                props!(
                    width: MENU_WIDTH,
                    gap: MENU_GAP,
                    padding: MENU_PADDING,
                    background: GRAY_100
                ),
                buttons,
            )],
        )
    }

    fn notice_pill(message: &str) -> Node {
        row(
            props!(
                background: GRAY_100,
                padding: 6.0,
                cross_align: CrossAlign::Center
            ),
            [text(
                message.to_owned(),
                style!(
                    size: NOTICE_TEXT_SIZE,
                    weight: FontWeight::REGULAR,
                    color: GRAY_30,
                    line_height: 1.0
                ),
            )],
        )
    }

    /// Overlay the tap catcher (always) and the menu (when open) onto the root.
    fn with_interaction(
        mut root: Node,
        menu_open: bool,
        label_button: Option<&'static str>,
    ) -> Node {
        if let Node::Column(_, children) | Node::Row(_, children) = &mut root {
            children.push(tap_catcher());
            if menu_open {
                children.push(menu_panel(label_button));
            }
        }
        root
    }

    /// Count a timer down by a frame; true once it reaches zero.
    fn tick(timer: &'static std::thread::LocalKey<Cell<u32>>, delta_ms: u32) -> bool {
        let left = timer.with(Cell::get);
        if left == 0 {
            return false;
        }
        let left = left.saturating_sub(delta_ms);
        timer.with(|t| t.set(left));
        left == 0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(delta_ms: u32) {
        // First render restores from cache (init() has no renderer scope).
        if INITIAL_RESTORE.with(Cell::get) {
            INITIAL_RESTORE.with(|f| f.set(false));
            dispatch(restore_from_cache());
        }

        tick(&MENU_MS, delta_ms);
        if tick(&NOTICE_LEFT_MS, delta_ms) {
            NOTICE.set(None);
        }

        let size = widget_size();
        let shape = widget_viewport().shape;
        let mut base = if server().is_none() {
            render::message_view(CONFIGURE_SERVER, size)
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
                        Badge::Updating => with_overlay(view, render::updating_pill(), shape),
                        // is_stale adds the grace window, so the 10s retry heals a blip first.
                        Badge::Stale => match POLL.with_borrow(|p| {
                            p.as_ref()
                                .filter(|handle| handle.is_stale())
                                .and_then(PollHandle::last_success_time)
                        }) {
                            Some(anchor) => with_stale_overlay(view, anchor, shape),
                            None => view,
                        },
                        // A broken payload states its specific reason at once.
                        Badge::Error(kind) => {
                            with_error_overlay(view, render::error_message(*kind), shape)
                        }
                        Badge::Fresh => view,
                    }
                }
                View::Loading { .. } => {
                    render::message_view(PROBLEM.with(Cell::get).unwrap_or(render::LOADING), size)
                }
                View::Failed(kind) => render::message_view(render::error_message(*kind), size),
            })
        };
        if let Some(message) = NOTICE.with_borrow(Clone::clone) {
            base = with_overlay(base, notice_pill(&message), shape);
        }

        let open = menu_open();
        let label_button = label().is_some().then(|| {
            if labelled() {
                "Remove label"
            } else {
                "Label it"
            }
        });
        let result = render_ui(
            size.width,
            size.height,
            with_interaction(base, open, label_button),
        );

        // Route taps: a button when the menu is open, otherwise open/dismiss it.
        let changed = if open {
            if result.clicks.contains_key(render::KEY_RELOAD) {
                dispatch(Event::Reload);
                MENU_MS.with(|m| m.set(0));
                true
            } else if result.clicks.contains_key(KEY_LABEL) {
                MENU_MS.with(|m| m.set(0));
                send_label();
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

        // Re-render on a change; else hold one frame at the next deadline.
        let next = [MENU_MS.with(Cell::get), NOTICE_LEFT_MS.with(Cell::get)]
            .into_iter()
            .filter(|&ms| ms > 0)
            .min();
        if changed {
            request_frame();
        } else if let Some(ms) = next {
            request_frame_after(ms);
        }
    }
}

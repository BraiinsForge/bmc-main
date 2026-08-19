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

//! Filling the asset cache with the images the payloads point at.
//!
//! The producer behind [`crate::images::resolve`], carrying one image
//! at a time: the host refuses a second decode while one runs
//! (`max_image_decodes`), so nothing here need buffer fetched bytes
//! while waiting for a slot.
//!
//! # The cost: images arrive strictly one behind another
//!
//! One slow image delays every image after it,
//! and a screenful costs as many round trips as it has images.
//! All that bounds an unreachable URL is [`FETCH_TIMEOUT`],
//! which the whole queue waits out.
//! Overlapping the fetches would lift the ceiling,
//! at the price of buffering each fetched image
//! until the single decoder frees up.
//!
//! Nothing retries: a failed image stays out of the cache,
//! so the next payload poll offers it again.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

#[expect(
    clippy::wildcard_imports,
    reason = "widget code uses many SDK types, macros, and helpers"
)]
use bmc_wasm_sdk::*;

use crate::images::{self, ImageKind, tag_for};

/// Shorter than the polls' own timeout: a payload is worth waiting for,
/// an image is one of many that each wait out this whole span.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

struct Image {
    tag: String,
    kind: ImageKind,
    url: String,
}

/// The handle is what makes a late callback safe to ignore: one outlived
/// by [`resume`] still reports in, and taking its answer for the image
/// that replaced it would cache one image's pixels under another's tag,
/// carrying that other URL as their identity — so no later check could
/// tell they are the wrong pixels.
enum Stage {
    Fetching(FetchRequestId),
    Decoding(ImageJobId),
}

struct Flight {
    stage: Stage,
    image: Image,
}

thread_local! {
    static QUEUE: RefCell<VecDeque<Image>> = const { RefCell::new(VecDeque::new()) };
    /// The one image between its fetch and its decode.
    static IN_FLIGHT: RefCell<Option<Flight>> = const { RefCell::new(None) };
}

/// Rebuild the work list from the held payloads and keep it moving.
///
/// Derived rather than accumulated, so a driver who leaves the standings
/// takes their images out of it.
pub fn sync() {
    // Not in the cache yet, so nothing else here would rule it out —
    // and queueing it again would have the flight's own completion
    // start a second fetch of what it just cached.
    let under_way = IN_FLIGHT.with(|flight| {
        flight
            .borrow()
            .as_ref()
            .map(|carried| carried.image.tag.clone())
    });
    let queued: VecDeque<Image> = crate::live::with_data(|data| {
        let mut queued: VecDeque<Image> = VecDeque::new();
        for want in images::wanted(data) {
            if !want.url.is_present() || images::cached_size(want.kind, want.url).is_some() {
                continue;
            }
            let tag = tag_for(want.kind, want.url);
            if Some(&tag) == under_way.as_ref() || queued.iter().any(|image| image.tag == tag) {
                continue;
            }
            queued.push_back(Image {
                tag,
                kind: want.kind,
                url: want.url.as_str().to_owned(),
            });
        }
        queued
    });
    QUEUE.with(|queue| *queue.borrow_mut() = queued);
    pump();
}

/// Resume after dormancy, abandoning whatever was in flight across it.
///
/// The host reclaims a decode that finished while dormant without ever
/// dispatching its callback, so the image it carried would otherwise
/// hold the chain shut for good rather than merely be lost.
pub fn resume() {
    IN_FLIGHT.with(|flight| *flight.borrow_mut() = None);
    sync();
}

fn pump() {
    if IN_FLIGHT.with(|flight| flight.borrow().is_some()) {
        return;
    }
    let Some(image) = QUEUE.with(|queue| queue.borrow_mut().pop_front()) else {
        return;
    };
    // A refused request means the fetch slots are full of payload polls;
    // dropping the image leaves it for the next sync to offer again.
    let Some(request) = net::FetchRequest::get(&image.url)
        .timeout(FETCH_TIMEOUT)
        .send(on_fetched)
    else {
        return;
    };
    IN_FLIGHT.with(|flight| {
        *flight.borrow_mut() = Some(Flight {
            stage: Stage::Fetching(request),
            image,
        });
    });
}

/// Whether the flight under way is the one this callback belongs to.
fn carrying(recognise: impl Fn(&Flight) -> bool) -> bool {
    IN_FLIGHT.with(|flight| flight.borrow().as_ref().is_some_and(recognise))
}

/// Holds the flight open across the decode, so nothing else starts.
fn on_fetched(response: &FetchResponse) {
    if !carrying(
        |flight| matches!(flight.stage, Stage::Fetching(request) if request == response.request_id),
    ) {
        return;
    }
    let decoding = IN_FLIGHT.with(|flight| {
        let mut flight = flight.borrow_mut();
        let carried = flight.as_mut()?;
        if !response.ok() || response.body().is_empty() {
            log_warn!(
                "formula-1: image fetch failed with status {}: {}",
                response.status,
                carried.image.url.as_str()
            );
            return None;
        }
        let (max_w, max_h) = carried.image.kind.decode_size();
        let job = set_bitmap_fit(
            &carried.image.tag,
            response.body(),
            max_w,
            max_h,
            false,
            carried.image.url.as_bytes(),
            on_decoded,
        )?;
        carried.stage = Stage::Decoding(job);
        Some(job)
    });
    if decoding.is_none() {
        release();
    }
}

fn on_decoded(job: ImageJobId, bitmap: Option<BitmapId>) {
    if !carrying(|flight| matches!(flight.stage, Stage::Decoding(pending) if pending == job)) {
        return;
    }
    if bitmap.is_some() {
        request_frame();
    }
    release();
}

fn release() {
    IN_FLIGHT.with(|flight| *flight.borrow_mut() = None);
    pump();
}

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

//! The decode → cache → restore path, as free functions keyed on a caller-chosen
//! cache identity. No poll, param or URL knowledge lives here.
//!
//! # Identity
//!
//! The identity is an opaque string the widget derives from everything that
//! makes a cached blob distinct — the request URL and the fit, at least. The
//! host stores it verbatim in the entry metadata, so [`stored_identity`] reads
//! back what the cached picture was fetched for without touching the payload.
//!
//! # Render scope
//!
//! [`restore`] and [`wake`] register a bitmap from the flash cache,
//! which the host serves through `with_renderer_and_state`.
//! That **traps outside a render scope**, killing the widget.
//! Lifecycle hooks are only delivered with the renderer parked,
//! so `render` and `on_wake` are legal; `init` and `on_params_update` are not.

#[expect(
    clippy::wildcard_imports,
    reason = "the picture pipeline uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::machine::{ErrorKind, Event};
use crate::render::{Fit, aspect_of};

/// Displayed picture; `set_fit_ref` evicts the previous.
///
/// The name is the flash cache key. It is deliberately still `image`: renaming
/// it would make every deployed widget miss its cache once and re-download.
static IMAGE: BitmapSlot = BitmapSlot::new("image");

/// Probe a fetch reply and, when it holds a usable picture, start the decode.
///
/// The request must have been built with `FetchSpec::host_body`, so the
/// encoded picture never crosses into guest memory.
///
/// A 2xx body that arrived whole and cannot be used is non-transient: it says
/// the same thing however often it is re-asked, so the widget names the reason
/// in a badge and backs the poll off rather than wearing a stale tag on the
/// fast `retry_ms` loop. Reaching nothing at all is transient.
#[must_use]
pub fn classify_body(
    response: &FetchResponse,
    size: WidgetSize,
    fit: Fit,
    identity: &str,
    on_ready: ImageReadyCallback,
) -> Event {
    let unusable = |kind: ErrorKind| Event::FetchError {
        kind,
        transient: false,
    };

    // Ahead of the ok() check below, which would read a refusal as transient.
    if response.outcome() == Some(FetchOutcome::BodyTooLarge) {
        return unusable(ErrorKind::TooLarge);
    }
    let body = response
        .body_ref()
        .expect("BUG: picture fetch did not retain its response body");
    if !response.ok() || body.is_empty() {
        return Event::FetchError {
            kind: ErrorKind::LoadFailed,
            transient: true,
        };
    }
    let Some(dimensions) = host::image_dimensions_ref(&body) else {
        return unusable(ErrorKind::BadImage);
    };
    let (w, h) = (dimensions.width, dimensions.height);
    if u64::from(w) * u64::from(h) > dimensions.max_source_pixels {
        return unusable(ErrorKind::TooLarge);
    }
    match IMAGE.set_fit_ref(
        body,
        size.width,
        size.height,
        fit == Fit::Cover,
        identity.as_bytes(),
        on_ready,
    ) {
        Ok(job) => Event::DecodeStarted {
            job,
            aspect: aspect_of(w, h),
        },
        // Decode slots full — retry later.
        Err(_body) => Event::FetchError {
            kind: ErrorKind::LoadFailed,
            transient: true,
        },
    }
}

/// Restore the cached picture when its identity matches. Render scope only.
#[must_use]
pub fn restore(identity: &str, interval_ms: u32) -> Event {
    let Some(state) = cached_state(identity, interval_ms) else {
        return Event::RestoreMiss;
    };
    let Some(bitmap) = assets::register_image(cache::lazy_get(IMAGE.name())) else {
        return Event::RestoreMiss;
    };
    let aspect = aspect_of(state.width, state.height);
    if state.remaining_ms > 0 {
        Event::Restored {
            bitmap,
            aspect,
            remaining_ms: state.remaining_ms,
            saved_at_secs: state.saved_at_secs,
        }
    } else {
        Event::RestoredStale {
            bitmap,
            aspect,
            saved_at_secs: state.saved_at_secs,
        }
    }
}

/// Re-arm a picture the widget still holds after dormancy. Render scope only.
#[must_use]
pub fn wake(identity: &str, interval_ms: u32) -> Event {
    let Some(state) = cached_state(identity, interval_ms) else {
        return Event::RestoreMiss;
    };
    if state.remaining_ms > 0 {
        Event::Woke {
            remaining_ms: state.remaining_ms,
            saved_at_secs: state.saved_at_secs,
        }
    } else {
        Event::WokeStale {
            saved_at_secs: state.saved_at_secs,
        }
    }
}

/// The identity the cached picture was fetched for, without reading the payload.
///
/// Lets a widget restore before it knows its current target —
/// the answer is on flash, not a round trip away.
#[must_use]
pub fn stored_identity() -> Option<String> {
    let stat = cache::stat(IMAGE.name())?;
    let (_, _, id_bytes) = parse_meta(&stat.metadata)?;
    core::str::from_utf8(id_bytes).ok().map(str::to_owned)
}

struct CachedState {
    width: u32,
    height: u32,
    remaining_ms: u32,
    saved_at_secs: i64,
}

fn cached_state(identity: &str, interval_ms: u32) -> Option<CachedState> {
    let stat = cache::stat(IMAGE.name())?;
    let (width, height, id_bytes) = parse_meta(&stat.metadata)?;
    if id_bytes != identity.as_bytes() {
        return None;
    }
    Some(CachedState {
        width,
        height,
        remaining_ms: remaining_ttl_ms(stat.saved_at, interval_ms),
        saved_at_secs: i64::try_from(stat.saved_at / 1000).unwrap_or(i64::MAX),
    })
}

// Cache metadata is `[w u32 | h u32 | identity]`, written by the host's
// `register_bitmap_from_cache` path — keep this in step with it.
fn parse_meta(meta: &[u8]) -> Option<(u32, u32, &[u8])> {
    let w = u32::from_le_bytes(meta.get(0..4)?.try_into().ok()?);
    let h = u32::from_le_bytes(meta.get(4..8)?.try_into().ok()?);
    Some((w, h, meta.get(8..)?))
}

fn remaining_ttl_ms(saved_at_ms: u64, interval_ms: u32) -> u32 {
    let now_ms = u64::try_from(host::SystemTime::now().unix_secs)
        .unwrap_or(0)
        .saturating_mul(1000);
    crate::machine::ttl_remaining(now_ms, saved_at_ms, interval_ms)
}

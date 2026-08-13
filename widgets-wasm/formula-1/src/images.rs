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

//! The widget's remote images, resolved out of the asset cache.
//!
//! Every image the payloads point at is cached as decoded RGBA under a
//! URL-derived tag, with the URL as the entry's identity. [`resolve`]
//! turns a URL back into a drawable bitmap — the deck restores what its
//! fetch pipeline cached, the storybook what a story seeded — and the
//! screens stay agnostic about which of the two filled the cache.

use std::cell::RefCell;
use std::collections::HashMap;

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "widget code uses many SDK types, macros, and helpers"
    )
)]
use bmc_wasm_sdk::*;

use crate::model::ImageUrl;

/// What an image shows, which sizes its decode and names its tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Headshot,
    TeamLogo,
    Flag,
    Circuit,
}

impl ImageKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Headshot => "headshot",
            Self::TeamLogo => "team-logo",
            Self::Flag => "flag",
            Self::Circuit => "circuit",
        }
    }

    /// The decode bound: the largest box the design draws the kind in,
    /// never the viewport, which would cache pixels nothing shows.
    #[must_use]
    pub fn decode_size(self) -> (u32, u32) {
        match self {
            Self::Headshot => (327, 330),
            Self::TeamLogo => (40, 40),
            Self::Flag => (40, 28),
            Self::Circuit => (560, 380),
        }
    }
}

/// The cache tag for `url`: stable across restarts, so a wake finds
/// what the previous run cached, and free of the separators cache keys
/// reject.
#[must_use]
pub fn tag_for(kind: ImageKind, url: &ImageUrl) -> String {
    fmt!("f1-{}-{}", kind.as_str(), fnv1a(url.as_str().as_bytes()))
}

/// FNV-1a, chosen over the std hasher because the tag persists on the
/// deck's flash — std documents its output as unstable across releases,
/// which would orphan every cached image on a toolchain bump.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// A resolved image: its bitmap and the pixel size it was decoded at,
/// for boxes that follow the image's aspect rather than fix their own.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub bitmap: BitmapId,
    pub width: u32,
    pub height: u32,
}

thread_local! {
    /// Resolved bitmaps by tag. Restoring uploads a texture, so a
    /// per-frame call must answer from here rather than re-upload.
    static RESOLVED: RefCell<HashMap<String, Resolved>> = RefCell::new(HashMap::new());
}

/// The bitmap for `url`, if its image is in the cache.
///
/// `None` while nothing has arrived — the screens hold a placeholder —
/// and for an entry whose identity is not this URL,
/// which a hash collision or a stale entry under a reused tag produces.
#[must_use]
pub fn resolve(kind: ImageKind, url: &ImageUrl) -> Option<Resolved> {
    if !url.is_present() {
        return None;
    }
    let tag = tag_for(kind, url);
    if let Some(hit) = RESOLVED.with(|memo| memo.borrow().get(&tag).copied()) {
        return Some(hit);
    }
    let stat = cache::stat(&tag)?;
    let (width, height, identity) = decode_image_meta(&stat.metadata)?;
    if identity != url.as_str().as_bytes() {
        return None;
    }
    let bitmap = assets::register_image(cache::lazy_get(&tag))?;
    let resolved = Resolved {
        bitmap,
        width,
        height,
    };
    RESOLVED.with(|memo| memo.borrow_mut().insert(tag, resolved));
    Some(resolved)
}

/// Forget every resolved bitmap. A wake from dormancy invalidates the
/// deck's bitmap ids, so the glue calls this before its first render.
pub fn invalidate_all() {
    RESOLVED.with(|memo| memo.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::{assets, cache, encode_image_meta};

    use super::{ImageKind, invalidate_all, resolve, tag_for};
    use crate::model::ImageUrl;

    fn url(s: &str) -> ImageUrl {
        ImageUrl::from(s.to_owned())
    }

    fn seed(kind: ImageKind, url: &ImageUrl) {
        let meta = encode_image_meta(2, 2, url.as_str().as_bytes());
        cache::put(&tag_for(kind, url), &meta, &[0_u8; 16]);
    }

    #[test]
    fn a_seeded_url_resolves() {
        assets::init_test_registrars();
        invalidate_all();
        let flag = url("https://cdn.example.test/flag/nl.png");
        seed(ImageKind::Flag, &flag);
        assert!(resolve(ImageKind::Flag, &flag).is_some());
    }

    #[test]
    fn an_absent_url_resolves_to_nothing() {
        assets::init_test_registrars();
        invalidate_all();
        assert!(resolve(ImageKind::Flag, &ImageUrl::default()).is_none());
        assert!(resolve(ImageKind::Flag, &url("https://x.test/missing.png")).is_none());
    }

    #[test]
    fn anothers_entry_under_the_tag_is_not_this_image() {
        assets::init_test_registrars();
        invalidate_all();
        let wanted = url("https://x.test/new.png");
        let meta = encode_image_meta(2, 2, b"https://x.test/old.png");
        cache::put(&tag_for(ImageKind::Headshot, &wanted), &meta, &[0_u8; 16]);
        assert!(resolve(ImageKind::Headshot, &wanted).is_none());
    }

    #[test]
    fn kinds_of_one_url_cache_apart() {
        let shared = url("https://x.test/img.png");
        assert_ne!(
            tag_for(ImageKind::Flag, &shared),
            tag_for(ImageKind::TeamLogo, &shared)
        );
    }
}

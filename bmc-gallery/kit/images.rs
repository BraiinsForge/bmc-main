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

//! Remote images for scenes, seeded from bytes the scene ships.
//!
//! Off-device there is no host fetching and decoding a widget's remote
//! images, so a scene seeds the SDK's cache with committed sample bytes
//! and the widget's own restore path draws them — the same
//! `stat` → identity check → `register_image` it runs on the deck.
//! Deterministic by construction: nothing touches the network.

use bmc_render::decode_scaled_to_fit;
use bmc_wasm_sdk::{cache, decode_image_meta, encode_image_meta};

/// Seed the cache entry a widget restores `tag` from, unless it already
/// holds this identity. Decodes to fit `max_w`×`max_h`, as the host does
/// on the deck; a scene calls this every frame and pays once.
pub fn seed_image(tag: &str, encoded: &[u8], max_w: u32, max_h: u32, identity: &[u8]) {
    let already_seeded = cache::stat(tag)
        .and_then(|stat| decode_image_meta(&stat.metadata).map(|(_, _, id)| id == identity))
        .unwrap_or(false);
    if already_seeded {
        return;
    }
    let Ok((rgba, width, height)) = decode_scaled_to_fit(encoded, max_w, max_h) else {
        return;
    };
    cache::put(tag, &encode_image_meta(width, height, identity), &rgba);
}

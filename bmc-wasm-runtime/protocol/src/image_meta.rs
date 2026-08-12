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

//! The metadata a cached image travels with.
//!
//! A decoded image's cache entry holds raw RGBA as its payload and this
//! layout as its metadata: the dimensions the restore needs, then the
//! identity — opaque caller bytes, typically the source URL and sizing —
//! that tells a restore whether the entry is still the wanted image.

/// The dimensions leading the metadata.
const DIMS_LEN: usize = 8;

/// Pack an image's dimensions and identity into cache metadata.
#[must_use]
pub fn encode_image_meta(width: u32, height: u32, identity: &[u8]) -> Vec<u8> {
    let mut meta = Vec::with_capacity(DIMS_LEN + identity.len());
    meta.extend_from_slice(&width.to_le_bytes());
    meta.extend_from_slice(&height.to_le_bytes());
    meta.extend_from_slice(identity);
    meta
}

/// Read `(width, height, identity)` back out of cache metadata.
///
/// `None` when the metadata is too short to carry the dimensions.
#[must_use]
pub fn decode_image_meta(meta: &[u8]) -> Option<(u32, u32, &[u8])> {
    let dims = meta.get(..DIMS_LEN)?;
    let width = u32::from_le_bytes([dims[0], dims[1], dims[2], dims[3]]);
    let height = u32::from_le_bytes([dims[4], dims[5], dims[6], dims[7]]);
    Some((width, height, &meta[DIMS_LEN..]))
}

#[cfg(test)]
mod tests {
    use super::{decode_image_meta, encode_image_meta};

    #[test]
    fn meta_round_trips() {
        let meta = encode_image_meta(640, 480, b"https://cdn/x.png");
        assert_eq!(
            decode_image_meta(&meta),
            Some((640, 480, b"https://cdn/x.png".as_slice()))
        );
    }

    #[test]
    fn an_empty_identity_round_trips() {
        let meta = encode_image_meta(1, 1, b"");
        assert_eq!(decode_image_meta(&meta), Some((1, 1, b"".as_slice())));
    }

    #[test]
    fn short_metadata_is_refused() {
        assert_eq!(decode_image_meta(&[0; 7]), None);
    }
}

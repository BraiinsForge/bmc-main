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

//! Compile-time embedding of binary assets, checked while compiling.
//!
//! An LFS-tracked asset is a text pointer file in a checkout that never fetched the object;
//! [`include_png!`](crate::include_png) and [`include_lfs!`](crate::include_lfs) reject those while compiling,
//! instead of leaving a decode error to happen on the device.
//! A PNG is checked against its signature;
//! a format without one only against the pointer's own prefix.
//! The checks are public because the macros' expansions call them from the invoking crate.

/// Check whether provided bytes contain PNG signature at the beginning.
#[must_use]
pub const fn is_png(bytes: &[u8]) -> bool {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    starts_with(bytes, &SIG)
}

/// `<[u8]>::starts_with`, which is not `const`.
const fn starts_with(bytes: &[u8], prefix: &[u8]) -> bool {
    if bytes.len() < prefix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if bytes[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Embed a PNG file's bytes, failing the build when the file is not a PNG.
///
/// Stands in for [`include_bytes!`] — `$path` resolves relative to the
/// invoking source file just the same — and additionally checks the PNG
/// signature while compiling. Images live in Git LFS (see `.gitattributes`),
/// and a checkout made without its LFS objects keeps a short text pointer in
/// the asset's place; plain `include_bytes!` embeds that pointer without
/// complaint, so the breakage surfaces far from its cause, as a decode error
/// on the device.
///
/// ```ignore
/// const DECK_LOGO_PNG: &[u8] = include_png!("../assets/deck_logo.png");
/// ```
#[macro_export]
macro_rules! include_png {
    ($path: literal) => {{
        const FILE_BYTES: &[u8] = ::core::include_bytes!($path);
        const _: () = ::core::assert!(
            $crate::asset::is_png(FILE_BYTES),
            ::core::concat!(
                "Not a PNG: '",
                $path,
                "' - it is probably a git-lfs pointer stub; run `git lfs pull`"
            )
        );
        FILE_BYTES
    }};
}

/// Whether `bytes` are a git-lfs pointer standing in for an object the checkout never fetched.
#[must_use]
pub const fn is_lfs_pointer(bytes: &[u8]) -> bool {
    starts_with(bytes, b"version https://git-lfs.github.com/spec/")
}

/// Embed an LFS-tracked file's bytes, failing the build when the checkout holds its pointer instead:
/// [`include_png!`](crate::include_png) for a format with no signature to check, such as an ICU4X blob.
///
/// ```ignore
/// const BASE: &[u8] = include_lfs!("../../../assets/intl/cldr-base.postcard");
/// ```
#[macro_export]
macro_rules! include_lfs {
    ($path: literal) => {{
        const FILE_BYTES: &[u8] = ::core::include_bytes!($path);
        const _: () = ::core::assert!(
            !$crate::asset::is_lfs_pointer(FILE_BYTES),
            ::core::concat!(
                "A git-lfs pointer stub, not the file: '",
                $path,
                "'; run `git lfs pull`"
            )
        );
        FILE_BYTES
    }};
}

#[cfg(test)]
mod tests {
    use super::{is_lfs_pointer, is_png};

    const LFS_POINTER: &str = "version https://git-lfs.github.com/spec/v1\n\
                               oid sha256:72d28658ad6339e7c5b800047c40de69576e4f22cea45929646f60af56d9c027\n\
                               size 6767\n";

    /// Real header: the eight signature bytes are `b"\x89PNG\r\n\x1a\n"`.
    #[test]
    fn test_accepts_png_signature() {
        let bytes_with_signature: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0xde, 0xad, 0xbe, 0xef,
        ];

        assert!(is_png(bytes_with_signature));
    }

    /// Catches a comparison that gives up before the end of the signature.
    /// The fixture clears the length guard and matches every signature byte
    /// but the last, which holds `0x00` where `0x0a` belongs.
    #[test]
    fn test_rejects_signature_differing_in_last_byte() {
        let bytes_with_wrong_last_header_byte: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x00, 0xde, 0xad, 0xbe, 0xef,
        ];

        assert!(
            !is_png(bytes_with_wrong_last_header_byte),
            "is_png rejects bytes that diverge in the signature's last byte"
        );
    }

    /// The case the module exists for: git-lfs substitutes a short text
    /// pointer that opens with `version https://git-lfs.github.com/spec/v1`.
    #[test]
    fn test_rejects_git_lfs_pointer() {
        assert!(!is_png(LFS_POINTER.as_bytes()));
    }

    /// Guards the `bytes.len() < prefix.len()` early return — without it the
    /// comparison loop would index past the end of a short file.
    #[test]
    fn test_rejects_input_shorter_than_signature() {
        let bytes_with_truncated_header: &[u8] = &[0x89, 0x50, 0x4e, 0x47];

        assert!(!is_png(bytes_with_truncated_header));
    }

    #[test]
    fn test_rejects_empty_input() {
        let empty: &[u8] = &[];

        assert!(!is_png(empty));
    }

    #[test]
    fn test_recognises_git_lfs_pointer_but_not_a_blob() {
        assert!(is_lfs_pointer(LFS_POINTER.as_bytes()));
        assert!(
            !is_lfs_pointer(&[0x03, 0xc4, 0x01, 0x00]),
            "an ICU4X blob's head"
        );
        assert!(!is_lfs_pointer(b"version"), "shorter than the prefix");
    }
}

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

//! URL construction from a trusted base and untrusted path segments.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Append percent-encoded path segments to an already-formed, trusted base.
/// Exact dot segments are encoded because RFC 3986 otherwise treats them as path structure.
#[must_use]
pub fn join_path_segments(base: &str, segments: &[&str]) -> String {
    let mut url = base.to_owned();
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 || !url.ends_with('/') {
            url.push('/');
        }
        match *segment {
            "." => url.push_str("%2E"),
            ".." => url.push_str("%2E%2E"),
            segment => {
                for encoded in utf8_percent_encode(segment, PATH_SEGMENT_ENCODE_SET) {
                    url.push_str(encoded);
                }
            }
        }
    }
    url
}

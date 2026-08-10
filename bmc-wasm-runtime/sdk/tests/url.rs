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

use bmc_wasm_sdk::url::join_path_segments;

#[test]
fn joins_and_encodes_path_segments() {
    assert_eq!(
        join_path_segments("https://nexus/api", &["prices", "BTC/USD?x#y"]),
        "https://nexus/api/prices/BTC%2FUSD%3Fx%23y"
    );
    assert_eq!(
        join_path_segments("https://nexus/api/", &["AZaz09-._~", "Kč"]),
        "https://nexus/api/AZaz09-._~/K%C4%8D"
    );
}

#[test]
fn exact_dot_segments_cannot_become_path_structure() {
    assert_eq!(
        join_path_segments("https://nexus/api/", &[".", "..", "a.b"]),
        "https://nexus/api/%2E/%2E%2E/a.b"
    );
}

#[test]
fn empty_segments_keep_their_path_boundary() {
    assert_eq!(
        join_path_segments("https://nexus/api/", &["one", "", "two"]),
        "https://nexus/api/one//two"
    );
}

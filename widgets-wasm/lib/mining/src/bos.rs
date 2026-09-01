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

//! The slice of the BOS REST API every miner widget speaks:
//! where to log in, how to read the token back, and how to address an endpoint.
//!
//! Which statuses count as an auth failure is deliberately not here.
//! The fleet adapters treat 403 as one and the single-miner widgets do not,
//! and the other device families answer differently again.

// `fmt!` expands to a `uwrite!` that resolves `ufmt` in the caller's scope.
use bmc_wasm_sdk::ufmt;

use crate::hashboards::JsonLookup;

/// Login endpoint, relative to a miner's API base.
pub const LOGIN_PATH: &str = "/auth/login";

/// Join a miner's base URL to an endpoint path,
/// tolerating a slash on either side of the seam.
#[must_use]
pub fn endpoint(base: &str, path: &str) -> String {
    bmc_wasm_sdk::fmt!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Body of a login request. BOS authenticates the `root` account only.
#[must_use]
pub fn login_body(password: &str) -> String {
    bmc_wasm_sdk::fmt!(
        r#"{{"username":"root","password":"{}"}}"#,
        bmc_wasm_sdk::JsonStr(password)
    )
}

/// The bearer token a login reply carried, if it carried one.
#[must_use]
pub fn token(json: &(impl JsonLookup + ?Sized)) -> Option<String> {
    json.str("/token")
}

#[cfg(test)]
mod tests {
    use super::{endpoint, login_body};

    #[test]
    fn joins_base_url_and_path_once() {
        assert_eq!(endpoint("http://miner", "/api"), "http://miner/api");
        assert_eq!(endpoint("http://miner/", "/api"), "http://miner/api");
        assert_eq!(endpoint("http://miner/", "api"), "http://miner/api");
    }

    #[test]
    fn login_body_escapes_the_password() {
        assert_eq!(
            login_body(r#"a"b"#),
            r#"{"username":"root","password":"a\"b"}"#
        );
    }
}

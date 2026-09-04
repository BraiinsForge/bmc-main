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
//! the endpoint paths, the login exchange, and the auth state it leaves behind.
//!
//! Endpoint auth statuses are deliberately not here:
//! the fleet adapters treat 403 as one and the single-miner widgets do not,
//! and the other device families answer differently again.
//! The login exchange is BOS's own, so [`login_refused`] does live here.

// `fmt!` expands to a `uwrite!` that resolves `ufmt` in the caller's scope.
use bmc_wasm_sdk::ufmt;

use crate::hashboards::JsonLookup;

/// Login endpoint, relative to a miner's API base.
pub const LOGIN_PATH: &str = "/auth/login";

// Endpoints the widgets read, relative to a miner's API base.
// Listed in full, so the surface lives here rather than in per-widget slices.
pub const STATS_PATH: &str = "/miner/stats";
pub const HASHBOARDS_PATH: &str = "/miner/hw/hashboards";
pub const DETAILS_PATH: &str = "/miner/details";
pub const CONSTRAINTS_PATH: &str = "/configuration/constraints";
pub const COOLING_PATH: &str = "/cooling/state";
pub const NETWORK_PATH: &str = "/network/";

/// Join a miner's base URL to an endpoint path,
/// tolerating a slash on either side of the seam.
///
/// Deliberately concatenation rather than a URL operation,
/// since the base is expected to be `scheme://host[:port]/path`.
///
/// `None` for a base carrying a query or fragment,
/// which would swallow the path appended after it.
#[must_use]
pub fn endpoint(base: &str, path: &str) -> Option<String> {
    if base.contains('?') || base.contains('#') {
        // Refusing sends no request, so the log is where the reason shows.
        #[cfg(target_arch = "wasm32")]
        bmc_wasm_sdk::log_warn!(
            "miner URL carries a query or fragment, so {} is unreachable",
            path
        );
        return None;
    }
    Some(bmc_wasm_sdk::fmt!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    ))
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
pub fn parse_token(json: &(impl JsonLookup + ?Sized)) -> Option<String> {
    json.str("/token")
}

/// Whether the miner turned the login away, as against never answering it:
/// a rejected credential or a 2xx with no token, and nothing else.
#[must_use]
pub fn login_refused(outcome: Option<bmc_wasm_sdk::FetchOutcome>) -> bool {
    matches!(
        outcome,
        Some(bmc_wasm_sdk::FetchOutcome::Http(401 | 403 | 200..=299))
    )
}

/// Where a caller stands with the miner it polls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AuthState {
    #[default]
    NoToken,
    LoggingIn,
    Authenticated(String),
    // A login attempt completed and was rejected — distinct from `LoggingIn`
    // so a rejection is visible, while the login poll keeps retrying underneath.
    Failed,
    // The login never got an answer, so the miner refused nothing:
    // no password is in question, and the last readings still stand.
    Unreachable,
}

impl AuthState {
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        match self {
            Self::Authenticated(token) => Some(token),
            Self::NoToken | Self::LoggingIn | Self::Failed | Self::Unreachable => None,
        }
    }

    #[must_use]
    pub fn auth_header(&self) -> Option<String> {
        self.token()
            .map(|token| bmc_wasm_sdk::fmt!("Authorization: {token}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthState, endpoint, login_body, login_refused};
    use bmc_wasm_sdk::FetchOutcome;

    /// A miner that never answered has rejected no password,
    /// so the login backoff and the auth banner both answer the wrong question.
    #[test]
    fn only_a_miner_that_answered_can_refuse_the_login() {
        assert!(login_refused(Some(FetchOutcome::Http(401))));
        assert!(login_refused(Some(FetchOutcome::Http(403))));
        assert!(login_refused(Some(FetchOutcome::Http(200))));
        assert!(!login_refused(Some(FetchOutcome::Network)));
        assert!(!login_refused(Some(FetchOutcome::Http(500))));
        assert!(!login_refused(Some(FetchOutcome::Refused)));
        assert!(!login_refused(Some(FetchOutcome::Aborted)));
        assert!(!login_refused(Some(FetchOutcome::BodyTooLarge)));
        assert!(!login_refused(None));
    }

    #[test]
    fn joins_base_url_and_path_once() {
        let joined = Some("http://miner/api".to_owned());
        assert_eq!(endpoint("http://miner", "/api"), joined);
        assert_eq!(endpoint("http://miner/", "/api"), joined);
        assert_eq!(endpoint("http://miner/", "api"), joined);
    }

    /// A path appended after a query or fragment lands inside it,
    /// so the request would reach somewhere nobody asked for.
    #[test]
    fn refuses_a_base_carrying_a_query_or_fragment() {
        assert_eq!(endpoint("http://miner/api?token=abc", "/stats"), None);
        assert_eq!(endpoint("http://miner/api#frag", "/stats"), None);
    }

    #[test]
    fn only_an_authenticated_state_carries_a_header() {
        let mut auth = AuthState::default();
        assert_eq!(auth, AuthState::NoToken);
        assert_eq!(auth.auth_header(), None);
        assert_eq!(AuthState::LoggingIn.auth_header(), None);
        assert_eq!(AuthState::Failed.auth_header(), None);
        auth = AuthState::Authenticated("abc".to_owned());
        assert_eq!(auth.auth_header(), Some("Authorization: abc".to_owned()));
        assert_eq!(AuthState::NoToken.token(), None);
    }

    #[test]
    fn login_body_escapes_the_password() {
        assert_eq!(
            login_body(r#"a"b"#),
            r#"{"username":"root","password":"a\"b"}"#
        );
    }
}

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

/// The BOS API of the miner this display belongs to.
/// Local mode dials it whatever the URL param says,
/// so the device's own session never travels.
pub const LOCAL_API: &str = "http://localhost/api/v1";

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

/// Body of a login request.
/// The JSON escaping covers a literal username or password;
/// on credential placeholders it is a no-op,
/// since the host substitutes the values only after the body is built.
#[must_use]
pub fn login_body(username: &str, password: &str) -> String {
    bmc_wasm_sdk::fmt!(
        r#"{{"username":"{}","password":"{}"}}"#,
        bmc_wasm_sdk::JsonStr(username),
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

/// The placeholder strings a widget's codegen module names for its two slots,
/// so this crate stays ignorant of slot names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placeholders {
    pub token: &'static str,
    pub username: &'static str,
    pub password: &'static str,
}

/// Which account a widget authenticates with, from the two slots it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMode {
    /// Only the local slot is bound:
    /// no login, the token placeholder goes straight to [`LOCAL_API`].
    Local {
        token_placeholder: &'static str,
    },
    /// Only the remote slot is bound: log in to `url` and hold its session.
    Remote {
        url: String,
        username_placeholder: &'static str,
        password_placeholder: &'static str,
    },
    Unbound,
    Ambiguous,
}

impl AuthMode {
    #[must_use]
    pub fn derive(
        local_bound: bool,
        remote_bound: bool,
        remote_url: &str,
        placeholders: Placeholders,
    ) -> Self {
        match (local_bound, remote_bound) {
            (true, false) => Self::Local {
                token_placeholder: placeholders.token,
            },
            (false, true) => Self::Remote {
                url: remote_url.to_owned(),
                username_placeholder: placeholders.username,
                password_placeholder: placeholders.password,
            },
            (false, false) => Self::Unbound,
            (true, true) => Self::Ambiguous,
        }
    }

    /// The login request this mode needs, as `(url, body)`.
    #[must_use]
    pub fn login(&self) -> Option<(String, String)> {
        match self {
            Self::Remote {
                url,
                username_placeholder,
                password_placeholder,
            } => Some((
                endpoint(url, LOGIN_PATH)?,
                login_body(username_placeholder, password_placeholder),
            )),
            Self::Local { .. } | Self::Unbound | Self::Ambiguous => None,
        }
    }

    /// A miner request under this mode, as `(url, authorization header)`,
    /// or `None` while nothing may be sent.
    #[must_use]
    pub fn miner_request(&self, auth: &AuthState, path: &str) -> Option<(String, String)> {
        match self {
            Self::Local { token_placeholder } => Some((
                endpoint(LOCAL_API, path)?,
                bmc_wasm_sdk::fmt!("Authorization: {}", token_placeholder),
            )),
            Self::Remote { url, .. } => Some((endpoint(url, path)?, auth.auth_header()?)),
            Self::Unbound | Self::Ambiguous => None,
        }
    }

    /// Where the login stands the moment this mode takes effect.
    #[must_use]
    pub fn initial_auth(&self) -> AuthState {
        match self {
            Self::Remote { .. } => AuthState::LoggingIn,
            Self::Local { .. } | Self::Unbound | Self::Ambiguous => AuthState::NoToken,
        }
    }

    /// What a miner reply does to the auth state.
    /// Local mode has no login to consult,
    /// so the reply itself raises the banner on a 401,
    /// and only a success lowers it.
    /// Remote mode re-arms the login on a 401,
    /// except while the login is already backing off after a refusal:
    /// restarting it would reset that backoff.
    #[must_use]
    pub fn reply_action(&self, current: &AuthState, status: u32) -> ReplyAction {
        match self {
            Self::Local { .. } => match status {
                401 => ReplyAction::SetAuth(AuthState::Failed),
                200..=299 => ReplyAction::SetAuth(AuthState::NoToken),
                _ => ReplyAction::Keep,
            },
            Self::Remote { .. } if status == 401 && *current != AuthState::Failed => {
                ReplyAction::Relogin
            }
            Self::Remote { .. } | Self::Unbound | Self::Ambiguous => ReplyAction::Keep,
        }
    }
}

/// What a miner reply does to the auth state, decided by [`AuthMode::reply_action`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyAction {
    SetAuth(AuthState),
    Relogin,
    Keep,
}

#[cfg(test)]
mod tests {
    use super::{
        AuthMode, AuthState, LOCAL_API, Placeholders, ReplyAction, endpoint, login_body,
        login_refused,
    };
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
            login_body("root", r#"a"b"#),
            r#"{"username":"root","password":"a\"b"}"#
        );
    }

    const PLACEHOLDERS: Placeholders = Placeholders {
        token: "{{ credential.bos_local.token }}",
        username: "{{ credential.bos_remote.username }}",
        password: "{{ credential.bos_remote.password }}",
    };

    fn mode(local: bool, remote: bool) -> AuthMode {
        AuthMode::derive(local, remote, "http://10.0.0.5/api/v1", PLACEHOLDERS)
    }

    #[test]
    fn exactly_one_bound_slot_picks_a_mode_and_anything_else_refuses() {
        assert!(matches!(mode(true, false), AuthMode::Local { .. }));
        assert!(matches!(mode(false, true), AuthMode::Remote { .. }));
        assert_eq!(mode(false, false), AuthMode::Unbound);
        assert_eq!(mode(true, true), AuthMode::Ambiguous);
    }

    /// The device's own session must never travel to another miner:
    /// local mode dials the fixed local API whatever the URL param says.
    #[test]
    fn local_mode_has_no_login_and_sends_the_token_placeholder_to_localhost_only() {
        let local = mode(true, false);
        assert_eq!(local.login(), None);
        let (url, header) = local
            .miner_request(&AuthState::NoToken, "/miner/stats")
            .expect("BUG: local mode always has a request");
        assert_eq!(url, "http://localhost/api/v1/miner/stats");
        assert!(url.starts_with(LOCAL_API));
        assert_eq!(header, "Authorization: {{ credential.bos_local.token }}");
        assert_eq!(local.initial_auth(), AuthState::NoToken);
    }

    #[test]
    fn remote_mode_logs_in_with_both_placeholders_and_no_literal_username() {
        let remote = mode(false, true);
        let (url, body) = remote.login().expect("BUG: remote mode logs in");
        assert_eq!(url, "http://10.0.0.5/api/v1/auth/login");
        assert_eq!(
            body,
            r#"{"username":"{{ credential.bos_remote.username }}","password":"{{ credential.bos_remote.password }}"}"#
        );
        assert!(!body.contains("root"));
        assert_eq!(remote.initial_auth(), AuthState::LoggingIn);
    }

    #[test]
    fn remote_mode_sends_the_session_token_to_the_configured_url_once_it_has_one() {
        let remote = mode(false, true);
        assert_eq!(
            remote.miner_request(&AuthState::NoToken, "/miner/stats"),
            None
        );
        let (url, header) = remote
            .miner_request(&AuthState::Authenticated("sess".to_owned()), "/miner/stats")
            .expect("BUG: a token allows the request");
        assert_eq!(url, "http://10.0.0.5/api/v1/miner/stats");
        assert_eq!(header, "Authorization: sess");
    }

    #[test]
    fn unbound_and_ambiguous_send_nothing() {
        for mode in [mode(false, false), mode(true, true)] {
            assert_eq!(mode.login(), None);
            assert_eq!(
                mode.miner_request(&AuthState::Authenticated("sess".to_owned()), "/miner/stats"),
                None
            );
            assert_eq!(mode.initial_auth(), AuthState::NoToken);
        }
    }

    /// Local mode has no login poll to re-arm,
    /// so the reply itself raises the auth banner, and only a success lowers it:
    /// a 500 or a refused connection proves nothing about the token.
    #[test]
    fn local_mode_fails_on_401_and_recovers_on_success_only() {
        let local = mode(true, false);
        assert_eq!(
            local.reply_action(&AuthState::NoToken, 401),
            ReplyAction::SetAuth(AuthState::Failed)
        );
        assert_eq!(
            local.reply_action(&AuthState::Failed, 500),
            ReplyAction::Keep
        );
        assert_eq!(local.reply_action(&AuthState::Failed, 0), ReplyAction::Keep);
        assert_eq!(
            local.reply_action(&AuthState::Failed, 200),
            ReplyAction::SetAuth(AuthState::NoToken)
        );
    }

    /// A remote 401 re-arms the login once;
    /// while the login is already backing off after a refusal,
    /// further 401s must not restart it.
    #[test]
    fn remote_mode_relogs_in_on_401_unless_the_login_already_refused() {
        let remote = mode(false, true);
        let session = AuthState::Authenticated("sess".to_owned());
        assert_eq!(remote.reply_action(&session, 401), ReplyAction::Relogin);
        assert_eq!(
            remote.reply_action(&AuthState::Failed, 401),
            ReplyAction::Keep
        );
        assert_eq!(remote.reply_action(&session, 200), ReplyAction::Keep);
        assert_eq!(remote.reply_action(&session, 500), ReplyAction::Keep);
    }

    #[test]
    fn unbound_and_ambiguous_ignore_replies() {
        for mode in [mode(false, false), mode(true, true)] {
            assert_eq!(
                mode.reply_action(&AuthState::NoToken, 401),
                ReplyAction::Keep
            );
            assert_eq!(
                mode.reply_action(&AuthState::NoToken, 200),
                ReplyAction::Keep
            );
        }
    }
}

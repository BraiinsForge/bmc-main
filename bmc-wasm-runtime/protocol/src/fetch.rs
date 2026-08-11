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

//! How a fetch ended, as it crosses the host/widget seam.
//!
//! The `status` field of `__on_fetch_response` is one `u32` carrying two things:
//! what the origin answered, and what the host decided on the widget's behalf.
//! One enum over both is what stops a host refusal reading as an origin reply.
//!
//! Wire format: `Http` is the status code itself;
//! host outcomes take values above the 100–999 range an origin can send.
//! `from_wire` rejects everything else, so an outcome added
//! after a widget was built cannot be mistaken for a status code.

/// First wire value reserved for host outcomes, above every HTTP status code.
const HOST_OUTCOME_BASE: u32 = 1_000;

/// A match pattern cannot do arithmetic, so each outcome past the first is named.
const WIRE_REFUSED: u32 = HOST_OUTCOME_BASE + 1;
const WIRE_ABORTED: u32 = HOST_OUTCOME_BASE + 2;

/// Widest range `http::StatusCode` accepts. A narrower cap would misread
/// a genuine origin status as an unknown host outcome.
const HTTP_STATUS_MAX: u32 = 999;
const HTTP_STATUS_MIN: u32 = 100;

const _: () = assert!(
    HOST_OUTCOME_BASE > HTTP_STATUS_MAX,
    "a host outcome must not be encodable as an origin status"
);

/// How a fetch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOutcome {
    /// The origin answered.
    Http(u16),
    /// No answer at all — DNS, connect, TLS or timeout.
    Network,
    /// The host stopped reading: the body outgrew its cap. Retrying is futile.
    BodyTooLarge,
    /// The host never sent it: a credential would not resolve,
    /// or its type pins egress away from this destination. Retrying is futile.
    Refused,
    /// The widget cancelled it. Every request settles exactly once, so a
    /// cancelled one settles here rather than going quiet — the caller can
    /// tell "I stopped this" from "the origin never answered".
    Aborted,
}

impl FetchOutcome {
    #[must_use]
    pub const fn to_wire(self) -> u32 {
        match self {
            Self::Http(code) => code as u32,
            Self::Network => 0,
            Self::BodyTooLarge => HOST_OUTCOME_BASE,
            Self::Refused => WIRE_REFUSED,
            Self::Aborted => WIRE_ABORTED,
        }
    }

    /// Returns `None` for a wire value this build has no meaning for.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "guarded by the HTTP_STATUS_MIN..=HTTP_STATUS_MAX range check"
    )]
    pub const fn from_wire(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Network),
            HTTP_STATUS_MIN..=HTTP_STATUS_MAX => Some(Self::Http(raw as u16)),
            HOST_OUTCOME_BASE => Some(Self::BodyTooLarge),
            WIRE_REFUSED => Some(Self::Refused),
            WIRE_ABORTED => Some(Self::Aborted),
            _ => None,
        }
    }

    /// Whether the origin answered in the 2xx range.
    #[must_use]
    pub const fn is_ok(self) -> bool {
        matches!(self, Self::Http(200..=299))
    }
}

#[cfg(test)]
mod tests {
    use super::{FetchOutcome, HOST_OUTCOME_BASE};

    #[test]
    fn round_trips_every_outcome() {
        for outcome in [
            FetchOutcome::Http(200),
            FetchOutcome::Http(404),
            // Nonstandard, but `http::StatusCode` accepts it and origins do send it.
            FetchOutcome::Http(600),
            FetchOutcome::Network,
            FetchOutcome::BodyTooLarge,
            FetchOutcome::Refused,
            FetchOutcome::Aborted,
        ] {
            assert_eq!(FetchOutcome::from_wire(outcome.to_wire()), Some(outcome));
        }
    }

    #[test]
    fn network_stays_zero() {
        assert_eq!(
            FetchOutcome::Network.to_wire(),
            0,
            "widgets built before this enum read 0 as the network error"
        );
    }

    #[test]
    fn host_outcomes_cannot_collide_with_a_status_code() {
        assert_eq!(FetchOutcome::from_wire(999), Some(FetchOutcome::Http(999)));
        assert_eq!(
            FetchOutcome::from_wire(HOST_OUTCOME_BASE),
            Some(FetchOutcome::BodyTooLarge)
        );
    }

    /// The probe is the slot the next outcome would take, so claiming it means
    /// updating this — the reminder that a widget built before the addition
    /// reads it as unknown rather than as something else.
    #[test]
    fn unknown_outcomes_decode_to_none() {
        assert_eq!(FetchOutcome::from_wire(HOST_OUTCOME_BASE + 3), None);
        assert_eq!(FetchOutcome::from_wire(u32::MAX), None);
    }

    #[test]
    fn a_refusal_is_not_a_network_failure() {
        assert_ne!(
            FetchOutcome::Refused.to_wire(),
            FetchOutcome::Network.to_wire(),
            "a widget retries a network failure; a refusal can never succeed"
        );
    }

    #[test]
    fn only_2xx_is_ok() {
        assert!(FetchOutcome::Http(204).is_ok());
        assert!(!FetchOutcome::Http(404).is_ok());
        assert!(!FetchOutcome::Network.is_ok());
        assert!(!FetchOutcome::BodyTooLarge.is_ok());
    }
}

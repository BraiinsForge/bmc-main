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

//! What a price reply does to a ticker view's held state. Both ticker
//! widgets fold price replies through this one mapping, so the single
//! tile and the list rows cannot drift apart on how a 404, a refusal,
//! or a failure reads.

use crate::fetch::{FetchClass, PriceMiss};
use crate::reference::ReferenceOutcome;

/// What a price reply does to the view's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// Replace the held series with the freshly parsed one.
    Store,
    /// Leave the view unchanged, keeping the last-good data on screen.
    Keep,
    /// The symbol is not resolvable — render a "not found" placeholder
    /// while polling continues. Carries why the price reply was empty
    /// so a later reference verdict reconciles only what it can explain.
    InputError(PriceMiss),
    /// The instrument exists but this window carries no candles — a closed
    /// market serves none. Distinct from [`Transition::InputError`]
    /// so the view stops claiming the symbol is unknown.
    NoData,
    /// Transient failure with nothing loaded yet; the poll keeps running.
    Fail,
}

/// Which placeholder a view with nothing to draw carries, given the reason
/// the price reply was empty and the reference resource's settled answer.
/// Both the price path and the reference path route through this,
/// so a late reply on either side moves the view and strands no verdict.
#[must_use]
pub fn placeholder(miss: PriceMiss, reference: ReferenceOutcome) -> Transition {
    match miss {
        PriceMiss::Rejected => Transition::InputError(miss),
        PriceMiss::NotFound => match reference {
            ReferenceOutcome::Resolved => Transition::NoData,
            ReferenceOutcome::Unknown | ReferenceOutcome::NotFound => Transition::InputError(miss),
        },
    }
}

/// Fold an HTTP-status class and the parse result into a transition.
/// Held data survives any failed refresh; a 404 shows an input error unless
/// the reference resolved the instrument, which makes it no-data instead.
#[must_use]
pub fn from_reply(
    class: FetchClass,
    parsed_ok: bool,
    has_data: bool,
    reference: ReferenceOutcome,
) -> Transition {
    match PriceMiss::of(class) {
        Some(miss) => placeholder(miss, reference),
        None if class == FetchClass::Ok && parsed_ok => Transition::Store,
        // A 503 also covers "no data for this symbol". Without held data,
        // degrade the view like any failure; keep polling for recovery.
        None => {
            if has_data {
                Transition::Keep
            } else {
                Transition::Fail
            }
        }
    }
}

/// Whether a reply class warrants a fast `retry()` rather than waiting for
/// the next poll. A 2xx whose body failed to parse is worth retrying sooner;
/// other failure classes defer to the poll engine.
#[must_use]
pub fn should_retry(class: FetchClass) -> bool {
    class == FetchClass::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parsed_payload_stores_the_series() {
        assert_eq!(
            from_reply(FetchClass::Ok, true, false, ReferenceOutcome::Unknown),
            Transition::Store
        );
        assert_eq!(
            from_reply(FetchClass::Ok, true, true, ReferenceOutcome::Unknown),
            Transition::Store
        );
    }

    #[test]
    fn input_error_replaces_held_data() {
        for has_data in [false, true] {
            assert_eq!(
                from_reply(
                    FetchClass::InputError,
                    false,
                    has_data,
                    ReferenceOutcome::Unknown
                ),
                Transition::InputError(PriceMiss::Rejected)
            );
        }
    }

    #[test]
    fn a_resolved_reference_turns_a_price_404_into_no_data() {
        // A closed market serves no candles for the window
        // while the instrument itself still resolves — the view
        // must not claim the symbol is unknown.
        for has_data in [false, true] {
            assert_eq!(
                from_reply(
                    FetchClass::NotFound,
                    false,
                    has_data,
                    ReferenceOutcome::Resolved
                ),
                Transition::NoData
            );
        }
    }

    #[test]
    fn a_price_404_stays_not_found_until_the_reference_resolves() {
        for reference in [ReferenceOutcome::Unknown, ReferenceOutcome::NotFound] {
            assert_eq!(
                from_reply(FetchClass::NotFound, false, false, reference),
                Transition::InputError(PriceMiss::NotFound)
            );
        }
    }

    #[test]
    fn the_reference_path_reaches_the_same_verdict_as_the_price_path() {
        // A reference reply can land after the view already chose a message
        // from a price 404. Both paths read the same mapping, so a reference
        // that later resolves — or stops resolving — moves the view
        // instead of leaving it stuck on the verdict the other path reached.
        for reference in [
            ReferenceOutcome::Unknown,
            ReferenceOutcome::Resolved,
            ReferenceOutcome::NotFound,
        ] {
            assert_eq!(
                placeholder(PriceMiss::NotFound, reference),
                from_reply(FetchClass::NotFound, false, false, reference)
            );
            assert_eq!(
                placeholder(PriceMiss::Rejected, reference),
                from_reply(FetchClass::InputError, false, false, reference)
            );
        }
    }

    #[test]
    fn a_refused_request_is_never_reinterpreted_by_the_reference() {
        // A resolved instrument explains an empty window. It does not explain
        // a request Nexus refused, which no amount of instrument metadata
        // explains away — reporting it as a closed market hides a real fault.
        for reference in [
            ReferenceOutcome::Unknown,
            ReferenceOutcome::Resolved,
            ReferenceOutcome::NotFound,
        ] {
            assert_eq!(
                placeholder(PriceMiss::Rejected, reference),
                Transition::InputError(PriceMiss::Rejected)
            );
        }
    }

    #[test]
    fn any_failure_keeps_held_data_else_fails() {
        // Surface a 503 with nothing loaded as unavailable; it can also mean
        // no symbol data.
        for class in [FetchClass::Ok, FetchClass::Backoff, FetchClass::Transient] {
            assert_eq!(
                from_reply(class, false, true, ReferenceOutcome::Unknown),
                Transition::Keep
            );
            assert_eq!(
                from_reply(class, false, false, ReferenceOutcome::Unknown),
                Transition::Fail
            );
        }
    }

    #[test]
    fn should_retry_only_on_ok() {
        assert!(should_retry(FetchClass::Ok));
        assert!(!should_retry(FetchClass::Transient));
        assert!(!should_retry(FetchClass::Backoff));
        assert!(!should_retry(FetchClass::InputError));
    }
}

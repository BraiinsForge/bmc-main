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

//! Instrument metadata resource (`reference/<instrument>`).

use bmc_wasm_sdk::url::join_path_segments;

use crate::candle::JsonLookup;
use crate::fetch::FetchClass;
use crate::instrument::split_pair;

/// Build the full Nexus URL for an instrument's reference resource.
#[must_use]
pub fn reference_url(base: &str, symbol: &str) -> String {
    match split_pair(symbol) {
        Some((base_symbol, quote_symbol)) => {
            join_path_segments(base, &["reference", base_symbol, quote_symbol])
        }
        None => join_path_segments(base, &["reference", symbol]),
    }
}

/// What the reference resource says about an instrument's existence.
/// A resolved reference is what separates "this window has no data"
/// from "this symbol does not exist" when the price resource answers 404.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReferenceOutcome {
    /// No conclusive reply yet, either because none has arrived
    /// or because the instrument changed and the previous answer was dropped.
    #[default]
    Unknown,
    /// The instrument exists.
    Resolved,
    /// The instrument does not exist.
    NotFound,
}

/// What a reference reply settles about the instrument, or [`None`] when it
/// settles nothing and the previous answer must stand.
#[must_use]
pub fn reference_outcome(class: FetchClass) -> Option<ReferenceOutcome> {
    match class {
        FetchClass::Ok => Some(ReferenceOutcome::Resolved),
        FetchClass::InputError | FetchClass::NotFound => Some(ReferenceOutcome::NotFound),
        FetchClass::Backoff | FetchClass::Transient => None,
    }
}

/// Refresh cadence for the instrument reference resource. Metadata moves
/// slowly, so the poll interval and a settled failure both wait this long.
pub const REFERENCE_REFRESH_MS: u32 = 1_800_000;

/// How a reference reply defers its poll. [`None`] leaves the engine's own
/// schedule: the interval after a good reply, the fast retry after a blip.
/// A settled miss or an explicit backoff waits out the full reference
/// cadence — the engine's fast failure retry would otherwise hammer Nexus
/// with a lookup it has already answered conclusively.
#[must_use]
pub fn reference_reschedule(class: FetchClass) -> Option<u32> {
    class.uses_poll_interval().then_some(REFERENCE_REFRESH_MS)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstrumentReference {
    pub name: Option<String>,
    pub is_market_open: Option<bool>,
}

/// Parse the metadata used by the ticker widgets.
#[must_use]
pub fn parse_reference(json: &impl JsonLookup) -> InstrumentReference {
    let name = json
        .str("/data/name")
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    InstrumentReference {
        name,
        is_market_open: json.bool("/data/is_market_open"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candle::JsonLookup;
    use crate::fetch::FetchClass;
    use crate::reference::ReferenceOutcome as Outcome;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MapJson {
        strings: BTreeMap<String, String>,
        bools: BTreeMap<String, bool>,
    }
    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).cloned()
        }
        fn i64(&self, _: &str) -> Option<i64> {
            None
        }
        fn f64(&self, _: &str) -> Option<f64> {
            None
        }
        fn bool(&self, path: &str) -> Option<bool> {
            self.bools.get(path).copied()
        }
    }

    #[test]
    fn reference_url_uses_the_instrument_mapping() {
        assert_eq!(
            reference_url("https://nexus/api/v1/data/", "BTC-USD"),
            "https://nexus/api/v1/data/reference/BTC/USD"
        );
        assert_eq!(reference_url("b/", "^GSPC"), "b/reference/%5EGSPC");
        assert_eq!(reference_url("b/", "BTC/USD"), "b/reference/BTC%2FUSD");
        assert_eq!(reference_url("b/", ".."), "b/reference/%2E%2E");
    }

    #[test]
    fn name_is_trimmed_and_empty_becomes_none() {
        let mut json = MapJson::default();
        json.strings
            .insert("/data/name".into(), "  Apple Inc.  ".into());
        assert_eq!(parse_reference(&json).name, Some("Apple Inc.".to_owned()));

        let mut empty = MapJson::default();
        empty.strings.insert("/data/name".into(), "   ".into());
        assert_eq!(parse_reference(&empty).name, None);

        assert_eq!(parse_reference(&MapJson::default()).name, None);
    }

    #[test]
    fn a_reply_that_settles_the_question_answers_it_either_way() {
        assert_eq!(reference_outcome(FetchClass::Ok), Some(Outcome::Resolved));
        assert_eq!(
            reference_outcome(FetchClass::NotFound),
            Some(Outcome::NotFound)
        );
        // The reference resource has one job: say whether the instrument
        // exists. A rejected lookup answers that as firmly as a 404, unlike
        // the price resource where a 400 says nothing about the instrument.
        assert_eq!(
            reference_outcome(FetchClass::InputError),
            Some(Outcome::NotFound)
        );
    }

    #[test]
    fn a_transient_reply_leaves_the_previous_answer_standing() {
        // Retracting a known instrument because one lookup timed out
        // would make a live tile flap to "not found" on any blip.
        assert_eq!(reference_outcome(FetchClass::Backoff), None);
        assert_eq!(reference_outcome(FetchClass::Transient), None);
    }

    #[test]
    fn a_settled_reference_miss_waits_out_the_reference_cadence() {
        // The poll engine reschedules any failed reply after its fast retry_ms
        // unless the handler defers it; a conclusive "no such instrument"
        // or an explicit backoff must wait the full reference cadence
        // instead of hammering Nexus every few seconds.
        for class in [
            FetchClass::InputError,
            FetchClass::NotFound,
            FetchClass::Backoff,
        ] {
            assert_eq!(reference_reschedule(class), Some(REFERENCE_REFRESH_MS));
        }
        // A good reply follows the poll interval and a transient failure
        // keeps the engine's fast retry, so a blip heals quickly.
        assert_eq!(reference_reschedule(FetchClass::Ok), None);
        assert_eq!(reference_reschedule(FetchClass::Transient), None);
    }

    #[test]
    fn market_state_is_read_from_reference_data() {
        let mut json = MapJson::default();
        assert_eq!(parse_reference(&json).is_market_open, None);
        json.bools.insert("/data/is_market_open".into(), false);
        assert_eq!(parse_reference(&json).is_market_open, Some(false));
    }
}

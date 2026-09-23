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

//! Nexus's `bitcoin/halving-prediction` envelope, read into a [`Prediction`].

use mining::hashboards::JsonLookup;

use crate::model::{Freshness, Prediction};

pub const URL: &str = "https://nexus.braiinsforge.com/api/v1/data/bitcoin/halving-prediction";

const BITCOIN_GENESIS_UNIX: i64 = 1_231_006_505;

fn unsigned(json: &(impl JsonLookup + ?Sized), path: &str) -> Option<u64> {
    json.i64(path).and_then(|value| u64::try_from(value).ok())
}

fn height(json: &(impl JsonLookup + ?Sized), path: &str) -> Option<u32> {
    json.i64(path).and_then(|value| u32::try_from(value).ok())
}

fn freshness(json: &(impl JsonLookup + ?Sized), received_at_secs: i64) -> Option<Freshness> {
    let ttl_secs = unsigned(json, "/ttl_secs")?;
    if ttl_secs == 0 {
        return None;
    }
    let cache_age_secs = unsigned(json, "/cache_age_secs")?;
    let payload_unix_secs =
        Some(received_at_secs.saturating_sub(i64::try_from(cache_age_secs).unwrap_or(i64::MAX)));
    Some(Freshness {
        payload_unix_secs,
        ttl_secs,
    })
}

/// The prediction and its freshness, or `None` when either is missing or implausible.
#[must_use]
pub fn parse(
    json: &(impl JsonLookup + ?Sized),
    parse_date: &impl Fn(&str) -> Option<i64>,
    received_at_secs: i64,
) -> Option<(Prediction, Freshness)> {
    let freshness = freshness(json, received_at_secs)?;
    let current_height = height(json, "/data/current/block_height")?;
    let target_block = height(json, "/data/next/block_height")?;
    if target_block == 0 {
        return None;
    }

    // Prefer the RFC3339 instant (stable across our local clock); fall back
    // to `next.delta` (seconds-from-now, recomputed server-side per read).
    let predicted_unix = json
        .str("/data/next/timestamp")
        .and_then(|timestamp| parse_date(&timestamp))
        .or_else(|| {
            json.i64("/data/next/delta")
                .and_then(|delta| received_at_secs.checked_add(delta))
        })?;
    if predicted_unix < BITCOIN_GENESIS_UNIX {
        return None;
    }

    Some((
        Prediction {
            current_height,
            target_block,
            predicted_unix,
        },
        freshness,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct MapJson {
        strings: BTreeMap<String, String>,
        ints: BTreeMap<String, i64>,
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).cloned()
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, _path: &str) -> Option<f64> {
            None
        }

        fn has(&self, path: &str) -> bool {
            let under = |key: &String| key.starts_with(path);
            self.strings.keys().any(under) || self.ints.keys().any(under)
        }
    }

    const NEXT_TIMESTAMP: &str = "2028-04-13T13:59:35Z";
    const NEXT_UNIX: i64 = 1_839_247_175;
    const RECEIVED_AT: i64 = 1_784_725_775;

    /// The live payload the bmc100 capture fixture recorded.
    fn recorded() -> MapJson {
        let mut json = MapJson::default();
        for (path, value) in [
            ("/ttl_secs", 60),
            ("/cache_age_secs", 57),
            ("/data/current/block_height", 959_131),
            ("/data/next/block_height", 1_050_000),
            ("/data/next/delta", 54_521_099),
        ] {
            json.ints.insert(path.to_owned(), value);
        }
        json.strings
            .insert("/data/next/timestamp".to_owned(), NEXT_TIMESTAMP.to_owned());
        json
    }

    fn parse_recorded_date(value: &str) -> Option<i64> {
        (value == NEXT_TIMESTAMP).then_some(NEXT_UNIX)
    }

    #[test]
    fn the_recorded_payload_reads_as_its_prediction_and_age() {
        let (prediction, freshness) = parse(&recorded(), &parse_recorded_date, RECEIVED_AT)
            .expect("BUG: the recorded payload is complete");
        assert_eq!(
            prediction,
            Prediction {
                current_height: 959_131,
                target_block: 1_050_000,
                predicted_unix: NEXT_UNIX,
            }
        );
        assert_eq!(
            freshness,
            Freshness {
                payload_unix_secs: Some(RECEIVED_AT - 57),
                ttl_secs: 60,
            }
        );
    }

    #[test]
    fn an_unreadable_timestamp_falls_back_to_the_delta() {
        let (prediction, _) = parse(&recorded(), &|_| None, RECEIVED_AT)
            .expect("BUG: the delta stands in for the timestamp");
        assert_eq!(prediction.predicted_unix, RECEIVED_AT + 54_521_099);
    }

    #[test]
    fn a_missing_field_rejects_the_payload() {
        for missing in [
            "/ttl_secs",
            "/cache_age_secs",
            "/data/current/block_height",
            "/data/next/block_height",
        ] {
            let mut json = recorded();
            json.ints.remove(missing);
            assert_eq!(
                parse(&json, &parse_recorded_date, RECEIVED_AT),
                None,
                "{missing}"
            );
        }
    }

    #[test]
    fn implausible_values_reject_the_payload() {
        for (path, value) in [
            ("/ttl_secs", 0),
            ("/data/next/block_height", 0),
            ("/data/current/block_height", -1),
            ("/data/next/block_height", i64::from(u32::MAX) + 1),
        ] {
            let mut json = recorded();
            json.ints.insert(path.to_owned(), value);
            assert_eq!(
                parse(&json, &parse_recorded_date, RECEIVED_AT),
                None,
                "{path} = {value}"
            );
        }
    }

    #[test]
    fn a_prediction_before_genesis_is_rejected() {
        let mut json = recorded();
        json.strings.remove("/data/next/timestamp");
        json.ints
            .insert("/data/next/delta".to_owned(), -RECEIVED_AT);
        assert_eq!(parse(&json, &parse_recorded_date, RECEIVED_AT), None);
    }
}

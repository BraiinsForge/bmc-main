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

//! Past states as `history/history_during_period` returns them.
//!
//! The result maps each entity id to an array in time order.
//! With `minimal_response` a state is `{"s": state, "lu": seconds}`,
//! the first also carrying what `no_attributes` leaves in.

use std::collections::BTreeMap;

use super::json::{Item, JsonLookup, Pointer, PointerBuf};

/// Most states kept from one reply, shared evenly between the entities that have any;
/// the latest of each are kept when there are more, and its series counts the rest as [`Series::dropped`].
///
/// Reading one costs the widget about 2,100 fuel,
/// so this leaves most of a delivery's 25 million to the widget.
pub const MAX_HISTORY_POINTS: usize = 1_000;

/// One entity's states over the requested period, oldest first,
/// as Home Assistant answered them but for the oldest the client dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct Series<P> {
    pub points: Vec<P>,
    /// How many of the oldest points were dropped to stay within [`MAX_HISTORY_POINTS`].
    /// Ask for a shorter period to see them.
    pub dropped: usize,
}

/// One state of an entity's history.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryPoint {
    /// When Home Assistant recorded the state, in seconds since the Unix epoch:
    /// when the entity took it, or, for a climate, humidifier or water heater,
    /// each later update of its attributes, which repeats the state.
    /// Home Assistant times the state the entity was in when the period began at its start,
    /// so a series that begins later has nothing earlier in the period.
    pub time: f64,
    /// The state as Home Assistant reports it, such as `21.5` or `unavailable`.
    pub state: String,
}

/// Each of `entity_ids` the result of `message` holds a series for, with its latest points:
/// together at most [`MAX_HISTORY_POINTS`], shared evenly between them.
fn read_all<T>(
    message: &impl JsonLookup,
    entity_ids: &[String],
    read: impl Fn(&str, usize) -> Option<Series<T>>,
) -> BTreeMap<String, Series<T>> {
    let answered: Vec<&String> = entity_ids
        .iter()
        .filter(|id| message.is_array(series_pointer(id).as_str()))
        .collect();
    let Some(most) = MAX_HISTORY_POINTS.checked_div(answered.len()) else {
        return BTreeMap::new();
    };
    answered
        .into_iter()
        .filter_map(|id| Some((id.clone(), read(id, most)?)))
        .collect()
}

/// The states of each of `entity_ids` in the result of `message`.
/// A state missing its time or its value is skipped.
pub(crate) fn read_history(
    message: &impl JsonLookup,
    entity_ids: &[String],
) -> BTreeMap<String, Series<HistoryPoint>> {
    read_all(message, entity_ids, |entity_id, most| {
        read_series(message, entity_id, most, |item| {
            Some(HistoryPoint {
                time: message.f64(item.key("lu"))?,
                state: message.str(item.key("s"))?,
            })
        })
    })
}

fn series_pointer(entity_id: &str) -> Pointer {
    Pointer::root().key("result").key(entity_id)
}

/// The latest `most` items of the result's array for `entity_id`,
/// each read by `read_item`, which skips one by returning `None`.
fn read_series<T>(
    message: &impl JsonLookup,
    entity_id: &str,
    most: usize,
    read_item: impl Fn(&mut Item<'_>) -> Option<T>,
) -> Option<Series<T>> {
    let mut path = PointerBuf::new(series_pointer(entity_id));
    if !message.is_array(path.as_str()) {
        return None;
    }
    let len = message.len(path.as_str())?;
    let first = len.saturating_sub(most);
    let array = path.mark();
    let mut points = Vec::with_capacity(len - first);
    for index in first..len {
        path.truncate(array);
        points.extend(read_item(&mut Item::new(&mut path, index)));
    }
    Some(Series {
        points,
        dropped: first,
    })
}

#[cfg(test)]
mod tests {
    use super::super::json::test_support::Message;
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    const START: i64 = 1_761_386_400;
    const OUTDOOR: &str = "sensor.outdoor";

    fn result(series: &serde_json::Value) -> Message {
        Message::parse(
            &serde_json::json!({"id": 3, "type": "result", "success": true, "result": series})
                .to_string(),
        )
    }

    fn ids(entity_ids: &[&str]) -> Vec<String> {
        entity_ids.iter().map(|&id| id.to_owned()).collect()
    }

    fn history(message: &Message, entity_id: &str) -> Option<Series<HistoryPoint>> {
        read_history(message, &ids(&[entity_id])).remove(entity_id)
    }

    fn states(series: &Series<HistoryPoint>) -> Vec<&str> {
        series
            .points
            .iter()
            .map(|point| point.state.as_str())
            .collect()
    }

    #[wasm_test]
    fn states_are_read_in_order_with_their_times() {
        let message = result(&serde_json::json!({OUTDOOR: [
            {"s": "12.4", "a": {"unit_of_measurement": "°C"}, "lu": 1_761_386_400.0},
            {"s": "12.6", "lu": 1_761_386_712.31},
            {"s": "unavailable", "lu": 1_761_387_000},
        ]}));

        assert_eq!(
            history(&message, OUTDOOR),
            Some(Series {
                points: vec![
                    HistoryPoint {
                        time: 1_761_386_400.0,
                        state: "12.4".to_owned()
                    },
                    HistoryPoint {
                        time: 1_761_386_712.31,
                        state: "12.6".to_owned()
                    },
                    HistoryPoint {
                        time: 1_761_387_000.0,
                        state: "unavailable".to_owned()
                    },
                ],
                dropped: 0,
            })
        );
    }

    #[wasm_test]
    fn a_state_missing_its_time_or_value_is_skipped() {
        let message = result(&serde_json::json!({OUTDOOR: [
            {"s": "1", "lu": 1.0},
            {"s": "2"},
            {"lu": 3.0},
            {"s": 4, "lu": 4.0},
            {"s": "5", "lu": 5.0},
        ]}));

        let series = history(&message, OUTDOOR).expect("BUG: the entity has a history");
        assert_eq!(states(&series), ["1", "5"]);
    }

    #[wasm_test]
    fn an_entity_the_result_leaves_out_has_no_history() {
        let message = result(&serde_json::json!({"sensor.other": []}));

        assert_eq!(history(&message, OUTDOOR), None);
    }

    #[wasm_test(fuel = 300e6, reason = "builds and parses 1,002 states in the guest")]
    fn past_the_cap_the_latest_states_are_kept_and_the_series_is_cut() {
        let states: Vec<serde_json::Value> = (0..MAX_HISTORY_POINTS + 2)
            .map(|second| serde_json::json!({"s": second.to_string(), "lu": START + 7 + i64::try_from(second).expect("BUG: a small count")}))
            .collect();
        let message = result(&serde_json::json!({ OUTDOOR: states }));

        let series = history(&message, OUTDOOR).expect("BUG: the entity has a history");
        assert_eq!(series.points.len(), MAX_HISTORY_POINTS);
        assert_eq!(series.points[0].state, "2", "the two oldest are dropped");
        assert_eq!(series.dropped, 2);
    }

    #[wasm_test(fuel = 300e6, reason = "builds and parses 1,000 states in the guest")]
    fn the_only_entity_that_answers_keeps_the_whole_share() {
        let states: Vec<serde_json::Value> = (0..MAX_HISTORY_POINTS)
            .map(|second| serde_json::json!({"s": second.to_string(), "lu": second}))
            .collect();
        let message = result(&serde_json::json!({ OUTDOOR: states }));

        let history = read_history(
            &message,
            &ids(&[OUTDOOR, "sensor.a", "sensor.b", "sensor.c"]),
        );
        assert_eq!(
            history.get(OUTDOOR).map(|series| series.points.len()),
            Some(MAX_HISTORY_POINTS),
            "the three left out take no share"
        );
    }

    #[wasm_test]
    fn a_result_answering_no_entity_holds_nothing() {
        let message = result(&serde_json::json!({}));

        assert!(read_history(&message, &ids(&[OUTDOOR])).is_empty());
    }
}

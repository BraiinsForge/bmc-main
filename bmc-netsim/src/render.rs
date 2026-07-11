// Copyright (C) 2026  Braiins Systems s.r.o.

//! Render a JSON body per request: any `{"$value": <spec>}` leaf is evaluated
//! to a number via [`Value`](crate::value::Value); everything else is copied.

use serde_json::Value as Json;

use crate::noise::{mix, mix_index};
use crate::value::Value;

const VALUE_MARKER: &str = "$value";

/// Render `body` at elapsed scenario time `t_s`, evaluating any dynamic leaves
/// with deterministic noise keyed on `seed`. Each object key and array index
/// folds into the seed as we descend, so sibling leaves decorrelate by path.
/// A body with no markers renders to a deep copy.
#[must_use]
pub fn render(body: &Json, t_s: f64, seed: u64) -> Json {
    match body {
        Json::Object(map) => {
            if map.len() == 1
                && let Some(spec) = map.get(VALUE_MARKER)
                && let Ok(value) = serde_json::from_value::<Value>(spec.clone())
            {
                return Json::from(value.eval(t_s, seed));
            }
            Json::Object(
                map.iter()
                    .map(|(key, val)| (key.clone(), render(val, t_s, mix(seed, key))))
                    .collect(),
            )
        }
        Json::Array(items) => Json::Array(
            items
                .iter()
                .enumerate()
                .map(|(index, val)| render(val, t_s, mix_index(seed, index)))
                .collect(),
        ),
        scalar @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::String(_)) => scalar.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::render;

    const SEED: u64 = 1;

    #[test]
    fn static_body_passes_through() {
        let body = json!({"a": 1, "b": ["x", true, null], "c": {"d": 2.5}});
        assert_eq!(render(&body, 0.0, SEED), body);
    }

    #[test]
    fn fixed_value_leaf_becomes_its_number() {
        let body = json!({"power": {"$value": {"kind": "fixed", "value": 3250.0}}});
        assert_eq!(render(&body, 0.0, SEED), json!({"power": 3250.0}));
    }

    #[test]
    fn ranged_leaf_stays_within_bounds() {
        let body = json!({"hr": {"$value": {"kind": "ranged", "min": 100.0, "max": 101.0}}});
        let out = render(&body, 0.0, SEED);
        let hr = out["hr"].as_f64().expect("BUG: rendered leaf is a number");
        assert!((100.0..101.0).contains(&hr), "{hr} out of range");
    }
}

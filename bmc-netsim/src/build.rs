// Copyright (C) 2026  Braiins Systems s.r.o.

//! Body-building helpers a device profile composes its endpoints from:
//! wrap a dynamic [`Value`] as a `$value` leaf, and the common value shapes.

use fake::Fake;
use fake::faker::internet::en::MACAddress;
use fake::rand::SeedableRng;
use fake::rand::rngs::StdRng;
use serde_json::{Value as Json, json};

use crate::noise::mix;
use crate::value::Value;

/// Wrap a dynamic [`Value`] as a `$value` leaf for a JSON body template.
#[must_use]
pub fn leaf(value: Value) -> Json {
    json!({ "$value": value })
}

/// A device's MAC from the `fake` faker, seeded by identity — stable per
/// device across runs, distinct across devices.
#[must_use]
pub fn mac(identity: &str) -> String {
    let mut rng = StdRng::seed_from_u64(mix(0, identity));
    MACAddress().fake_with_rng(&mut rng)
}

/// A value that drifts ~±2% around `center` over five minutes, lightly jittered.
#[must_use]
pub fn drift(center: f64) -> Value {
    Value::Drift {
        center,
        amp: center * 0.02,
        period_s: 300.0,
        jitter: center * 0.004,
    }
}

/// A temperature that drifts a few °C around `center`.
#[must_use]
pub fn celsius(center: f64) -> Value {
    Value::Drift {
        center,
        amp: 4.0,
        period_s: 180.0,
        jitter: 1.0,
    }
}

/// A constant value.
#[must_use]
pub fn steady(value: f64) -> Value {
    Value::Fixed { value }
}

/// A value that wanders smoothly in `[min, max)`, deterministic in `(seed, t)`.
#[must_use]
pub fn ranged(min: f64, max: f64) -> Value {
    Value::Ranged { min, max }
}

#[cfg(test)]
mod tests {
    use super::mac;

    #[test]
    fn mac_is_stable_and_distinct_per_identity() {
        assert_eq!(
            mac("bos-01"),
            mac("bos-01"),
            "same identity is reproducible"
        );
        assert_ne!(
            mac("bos-01"),
            mac("bos-02"),
            "distinct identities decorrelate"
        );
    }

    #[test]
    fn mac_has_six_hex_octets() {
        let m = mac("bos-01");
        let octets: Vec<&str> = m.split(':').collect();
        assert_eq!(octets.len(), 6, "{m} is six colon-separated octets");
        assert!(
            octets
                .iter()
                .all(|o| o.len() == 2 && o.bytes().all(|b| b.is_ascii_hexdigit())),
            "{m} octets are two hex digits",
        );
    }
}

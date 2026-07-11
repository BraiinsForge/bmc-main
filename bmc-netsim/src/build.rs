// Copyright (C) 2026  Braiins Systems s.r.o.

//! Body-building helpers a device profile composes its endpoints from:
//! wrap a dynamic [`Value`] as a `$value` leaf, and the common value shapes.

use serde_json::{Value as Json, json};

use crate::value::Value;

/// Wrap a dynamic [`Value`] as a `$value` leaf for a JSON body template.
#[must_use]
pub fn leaf(value: Value) -> Json {
    json!({ "$value": value })
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

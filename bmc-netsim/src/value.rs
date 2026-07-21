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

//! How one scalar telemetry value is produced: constant, smooth wander, or slow
//! drift — each a deterministic function of `(seed, t_s)`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::noise::noise01;

/// How a scalar value is produced on each read.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Value {
    /// A constant.
    Fixed { value: f64 },
    /// A smooth wander in `[min, max)`, deterministic in `(seed, t_s)`
    /// (yields `min` if the range is empty).
    Ranged { min: f64, max: f64 },
    /// A sine drift of amplitude `amp` around `center` over `period_s`,
    /// with optional deterministic `jitter` added on top.
    Drift {
        center: f64,
        amp: f64,
        period_s: f64,
        #[serde(default)]
        jitter: f64,
    },
}

impl Value {
    /// Evaluate at `t_s` seconds of elapsed scenario time, drawing any
    /// variation from deterministic noise keyed on `seed`.
    #[must_use]
    pub fn eval(&self, t_s: f64, seed: u64) -> f64 {
        match *self {
            Value::Fixed { value } => value,
            Value::Ranged { min, max } => {
                if max > min {
                    min + noise01(seed, t_s) * (max - min)
                } else {
                    min
                }
            }
            Value::Drift {
                center,
                amp,
                period_s,
                jitter,
            } => {
                let phase = if period_s > 0.0 {
                    (t_s / period_s) * std::f64::consts::TAU
                } else {
                    0.0
                };
                let base = center + amp * phase.sin();
                if jitter > 0.0 {
                    base + jitter * (noise01(seed, t_s) * 2.0 - 1.0)
                } else {
                    base
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    const SEED: u64 = 0x00C0_FFEE;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn fixed_is_constant() {
        let v = Value::Fixed { value: 42.0 };
        assert!(close(v.eval(0.0, SEED), 42.0));
        assert!(close(v.eval(100.0, SEED), 42.0));
    }

    #[test]
    fn ranged_stays_within_bounds() {
        let v = Value::Ranged { min: 1.0, max: 2.0 };
        for step in 0..1_000 {
            let x = v.eval(f64::from(step) * 0.1, SEED);
            assert!((1.0..2.0).contains(&x), "{x} out of range");
        }
    }

    #[test]
    fn ranged_empty_yields_min() {
        let v = Value::Ranged { min: 5.0, max: 5.0 };
        assert!(close(v.eval(0.0, SEED), 5.0));
    }

    #[test]
    fn eval_is_stable_at_same_time() {
        let v = Value::Ranged { min: 1.0, max: 2.0 };
        assert!(close(v.eval(7.5, SEED), v.eval(7.5, SEED)));
    }

    #[test]
    fn drift_hits_center_and_peak_without_jitter() {
        let v = Value::Drift {
            center: 10.0,
            amp: 2.0,
            period_s: 100.0,
            jitter: 0.0,
        };
        // t=0 → sine term 0 → exactly center.
        assert!(close(v.eval(0.0, SEED), 10.0));
        // Quarter period → sine 1 → center + amp.
        assert!(close(v.eval(25.0, SEED), 12.0));
    }
}

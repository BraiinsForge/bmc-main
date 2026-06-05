// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::telemetry::TelemetryReading;

const OK_HASHRATE_FLOOR_THS: f32 = 0.1;

#[must_use]
pub fn is_ok(reading: &TelemetryReading) -> bool {
    reading
        .current_hashrate_ths
        .is_some_and(|h| h > OK_HASHRATE_FLOOR_THS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(hashrate: Option<f32>) -> TelemetryReading {
        TelemetryReading {
            current_hashrate_ths: hashrate,
            ..TelemetryReading::default()
        }
    }

    #[test]
    fn ok_predicate_uses_the_hundred_gigahash_floor() {
        assert!(!is_ok(&reading(Some(0.1))), "exactly the floor is not ok");
        assert!(is_ok(&reading(Some(0.11))), "just above the floor is ok");
        assert!(!is_ok(&reading(None)), "no hashrate reading is not ok");
        assert!(!is_ok(&reading(Some(0.0))), "zero hashrate is not ok");
    }
}

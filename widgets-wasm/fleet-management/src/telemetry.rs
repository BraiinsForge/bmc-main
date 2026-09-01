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

use bmc_wasm_sdk::{Hashrate, Temperature};

/// A device's temperature at the fidelity its sensors provide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeviceTemp {
    /// One sensor (uBOS, single-board miners).
    Single(Temperature),
    /// Several sensors (BOS+ per hashboard, AxeOS `temp`/`temp2`).
    Spread {
        min: Temperature,
        avg: Temperature,
        max: Temperature,
    },
}

impl DeviceTemp {
    /// `(min, avg, max)`; a `Single` reports its one value for all three.
    #[must_use]
    pub fn as_range(self) -> (Temperature, Temperature, Temperature) {
        match self {
            Self::Single(t) => (t, t, t),
            Self::Spread { min, avg, max } => (min, avg, max),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TelemetryReading {
    pub current_hashrate_ths: Option<f32>,
    pub nominal_hashrate_ths: Option<f32>,
    pub power_w: Option<f32>,
    pub uptime_s: Option<u64>,
    pub temperature: Option<DeviceTemp>,
    /// Device MAC where the API exposes it — BOS+ and AxeOS, not uBOS.
    pub mac: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TelemetrySnapshot {
    pub reading: TelemetryReading,
}

/// A wire figure as a usable measurement, or `None` when it is not one.
/// Checked after the narrowing, since a magnitude beyond `f32::MAX`
/// only becomes infinite there — and an infinite reading walks the
/// chart's axis maths off the number line instead of scaling to anything.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "narrowing is the point here; the result is checked on the next line"
)]
pub fn measurement(value: f64) -> Option<f32> {
    let narrowed = value as f32;
    (narrowed.is_finite() && narrowed >= 0.0).then_some(narrowed)
}

/// A hashrate as the TH/s a reading stores. Each family builds the `Hashrate`
/// with whichever unit its firmware reports, so the scaling stays in one place.
#[must_use]
pub fn hashrate(rate: Hashrate) -> Option<f32> {
    measurement(rate.as_terahashes_per_second())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_keeps_a_figure_that_survives_narrowing() {
        assert_eq!(measurement(4.5), Some(4.5));
        assert_eq!(measurement(0.0), Some(0.0));
    }

    #[test]
    fn measurement_drops_what_is_not_a_reading() {
        // 1e300 is finite as f64 and only saturates once narrowed, which is why
        // the check has to come after the cast rather than on the wire figure.
        for bad in [1e300, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -1.0] {
            assert_eq!(measurement(bad), None, "{bad} is not a measurement");
        }
    }

    #[test]
    fn default_reading_keeps_all_fields_none() {
        let r = TelemetryReading::default();
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.nominal_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(r.uptime_s, None);
        assert_eq!(r.temperature, None);
        assert_eq!(r.mac, None);
    }
}

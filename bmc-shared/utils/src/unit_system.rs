// Copyright (C) 2025  Braiins Systems s.r.o.
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

use serde::{Deserialize, Serialize};

use crate::number_format::NumberFormat;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum UnitSystem {
    #[default]
    Metric,
    Imperial,
}

/// Which unit a metric-system speed reads in; imperial is always mph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MetricSpeedUnit {
    #[default]
    KmH,
    Ms,
}

impl UnitSystem {
    /// Metric keeps km/h or converts to m/s per `metric_unit`; imperial is mph.
    #[must_use]
    pub fn format_speed(
        self,
        number_format: NumberFormat,
        value_kmh: f64,
        precision: usize,
        metric_unit: MetricSpeedUnit,
    ) -> String {
        let (converted, suffix) = match (self, metric_unit) {
            (UnitSystem::Imperial, _) => (value_kmh * 0.621_371_192, " mph"),
            (UnitSystem::Metric, MetricSpeedUnit::KmH) => (value_kmh, " km/h"),
            (UnitSystem::Metric, MetricSpeedUnit::Ms) => (value_kmh / 3.6, " m/s"),
        };
        format!(
            "{}{suffix}",
            number_format.format_number(converted, precision)
        )
    }

    #[must_use]
    pub fn format_distance(
        self,
        number_format: NumberFormat,
        value_km: f64,
        precision: usize,
    ) -> String {
        let (converted, suffix) = match self {
            UnitSystem::Imperial => (value_km * 0.621_371_192, " mi"),
            UnitSystem::Metric => (value_km, " km"),
        };
        format!(
            "{}{suffix}",
            number_format.format_number(converted, precision)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_kmh_keeps_value_and_suffix() {
        let s = UnitSystem::Metric.format_speed(
            NumberFormat::SpaceGroupCommaDecimal,
            12.6,
            1,
            MetricSpeedUnit::KmH,
        );
        assert_eq!(s, "12,6 km/h");
    }

    #[test]
    fn metric_ms_divides_by_3_6() {
        let s = UnitSystem::Metric.format_speed(
            NumberFormat::SpaceGroupCommaDecimal,
            12.6,
            1,
            MetricSpeedUnit::Ms,
        );
        assert_eq!(s, "3,5 m/s");
    }

    #[test]
    fn imperial_speed_is_mph_regardless_of_metric_unit() {
        let kmh = UnitSystem::Imperial.format_speed(
            NumberFormat::SpaceGroupCommaDecimal,
            100.0,
            0,
            MetricSpeedUnit::KmH,
        );
        assert_eq!(kmh, "62 mph");
    }

    #[test]
    fn metric_distance_keeps_kilometres() {
        let d = UnitSystem::Metric.format_distance(NumberFormat::SpaceGroupDotDecimal, 420.0, 0);
        assert_eq!(d, "420 km");
    }

    #[test]
    fn imperial_distance_converts_to_miles() {
        let d = UnitSystem::Imperial.format_distance(NumberFormat::SpaceGroupDotDecimal, 100.0, 0);
        assert_eq!(d, "62 mi");
    }
}

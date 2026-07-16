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

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TelemetryReading {
    pub current_hashrate_ths: Option<f32>,
    pub nominal_hashrate_ths: Option<f32>,
    pub power_w: Option<f32>,
    pub uptime_s: Option<u64>,
    pub temperature_c: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TelemetrySnapshot {
    pub reading: TelemetryReading,
    pub refreshed_seq: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reading_keeps_all_fields_none() {
        let r = TelemetryReading::default();
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.nominal_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(r.uptime_s, None);
        assert_eq!(r.temperature_c, None);
    }
}

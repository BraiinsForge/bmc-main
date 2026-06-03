// Copyright (C) 2026  Braiins Systems s.r.o.

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

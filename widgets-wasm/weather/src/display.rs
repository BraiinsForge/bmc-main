// Copyright (C) 2026  Braiins Systems s.r.o.

#[cfg(target_arch = "wasm32")]
#[expect(clippy::wildcard_imports, reason = "widget code uses many SDK exports")]
use bmc_wasm_sdk::*;

pub const NOT_AVAILABLE: &str = "--";

#[must_use]
pub fn wind_line(direction: &str, speed: &str) -> String {
    let mut s = String::with_capacity("Wind From The ".len() + direction.len() + 1 + speed.len());
    s.push_str("Wind From The ");
    s.push_str(direction);
    s.push(' ');
    s.push_str(speed);
    s
}

#[must_use]
pub fn temperature_or_placeholder(value_c: Option<f64>, fmt: impl Fn(f64) -> String) -> String {
    value_c.map_or_else(|| NOT_AVAILABLE.to_string(), fmt)
}

#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn temperature(value_c: f64) -> String {
    format_temperature!(value_c, 0)
}

#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn wind_speed_ms(value_kmh: f64) -> String {
    format_speed!(value_kmh, 0, ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wind_line_reads_direction_then_speed() {
        assert_eq!(wind_line("South", "3 m/s"), "Wind From The South 3 m/s");
    }

    #[test]
    fn placeholder_used_when_value_absent() {
        assert_eq!(temperature_or_placeholder(None, |_| unreachable!()), "--");
    }

    #[test]
    fn value_present_runs_the_formatter() {
        assert_eq!(
            temperature_or_placeholder(Some(20.0), |_| "20".to_string()),
            "20"
        );
    }
}

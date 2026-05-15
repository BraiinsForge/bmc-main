// Copyright (C) 2025  Braiins Systems s.r.o.

//! Protocol type conversions.

use bmc_widget_protocol::server::deck_widget_surface_v1;
use bmc_widget_protocol::{
    DateFormat, NumberFormat, TemperatureUnit, TimeSystem, UnitSystem, WeekDay,
};

pub fn date_format_to_protocol(d: DateFormat) -> deck_widget_surface_v1::DateFormat {
    use deck_widget_surface_v1::DateFormat as P;
    match d {
        DateFormat::DdMmYyyyDot => P::DdMmYyyyDot,
        DateFormat::DdMmYyyySlash => P::DdMmYyyySlash,
        DateFormat::DMYyyySlash => P::DMYyyySlash,
        DateFormat::MDYyyySlash => P::MDYyyySlash,
        DateFormat::DdMmYyyyDash => P::DdMmYyyyDash,
        DateFormat::YyyyMDSlash => P::YyyyMDSlash,
        DateFormat::YyyyMmDdDot => P::YyyyMmDdDot,
        DateFormat::YyyyMmDdDash => P::YyyyMmDdDash,
    }
}

pub fn time_format_to_protocol(t: TimeSystem) -> deck_widget_surface_v1::TimeFormat {
    use deck_widget_surface_v1::TimeFormat as P;
    match t {
        TimeSystem::Hour12 => P::Hour12,
        TimeSystem::Hour24 => P::Hour24,
    }
}

pub fn number_format_to_protocol(n: NumberFormat) -> deck_widget_surface_v1::NumberFormat {
    use deck_widget_surface_v1::NumberFormat as P;
    match n {
        NumberFormat::SpaceGroupCommaDecimal => P::SpaceGroupCommaDecimal,
        NumberFormat::CommaGroupDotDecimal => P::CommaGroupDotDecimal,
        NumberFormat::DotGroupCommaDecimal => P::DotGroupCommaDecimal,
        NumberFormat::SpaceGroupDotDecimal => P::SpaceGroupDotDecimal,
    }
}

pub fn temperature_unit_to_protocol(u: TemperatureUnit) -> deck_widget_surface_v1::TemperatureUnit {
    use deck_widget_surface_v1::TemperatureUnit as P;
    match u {
        TemperatureUnit::Celsius => P::Celsius,
        TemperatureUnit::Fahrenheit => P::Fahrenheit,
    }
}

pub fn weekday_to_protocol(w: WeekDay) -> deck_widget_surface_v1::Weekday {
    use deck_widget_surface_v1::Weekday as P;
    match w {
        WeekDay::Monday => P::Monday,
        WeekDay::Tuesday => P::Tuesday,
        WeekDay::Wednesday => P::Wednesday,
        WeekDay::Thursday => P::Thursday,
        WeekDay::Friday => P::Friday,
        WeekDay::Saturday => P::Saturday,
        WeekDay::Sunday => P::Sunday,
    }
}

pub fn night_mode_to_protocol(enabled: bool) -> deck_widget_surface_v1::NightModeState {
    use deck_widget_surface_v1::NightModeState as P;
    if enabled { P::On } else { P::Off }
}

pub fn unit_system_to_protocol(u: UnitSystem) -> deck_widget_surface_v1::UnitSystem {
    use deck_widget_surface_v1::UnitSystem as P;
    match u {
        UnitSystem::Metric => P::Metric,
        UnitSystem::Imperial => P::Imperial,
    }
}

pub fn presence_to_protocol(present: bool) -> deck_widget_surface_v1::Presence {
    use deck_widget_surface_v1::Presence as P;
    if present { P::Present } else { P::Absent }
}

pub fn led_effect_from_protocol(
    e: deck_widget_surface_v1::LedEffect,
) -> bmc_widget_protocol::LedEffect {
    use bmc_widget_protocol::LedEffect as L;
    use deck_widget_surface_v1::LedEffect as P;
    match e {
        P::Chase => L::Chase,
        P::KnightRider => L::KnightRider,
        P::Scan => L::Scan,
        P::Snake => L::Snake,
        P::Breathe => L::Breathe,
        // Forward-compat: unknown values from a future protocol version
        // fall back to a benign Solid effect rather than panicking.
        P::Solid | _ => L::Solid,
    }
}

pub fn led_scope_from_protocol(
    s: deck_widget_surface_v1::LedScope,
) -> bmc_widget_protocol::LedScope {
    use bmc_widget_protocol::LedScope as L;
    use deck_widget_surface_v1::LedScope as P;
    match s {
        P::Global => L::Global,
        // `_` covers `Local` plus any future `#[non_exhaustive]` variant of the
        // generated enum, defaulting to the safe scene-scoped scope. (Unknown
        // wire values are already rejected upstream by `into_result`.)
        P::Local | _ => L::Local,
    }
}

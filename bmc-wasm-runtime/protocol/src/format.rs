// Copyright (C) 2026  Braiins Systems s.r.o.

//! Formatting preference types shared between the WASM SDK and host.
//!
//! Self-contained equivalents of the `LocalizationConfig` types from `bmc/src/config.rs`,
//! kept in the protocol crate so the WASM boundary has no dependency on bmc/bmc-shared.

/// Number formatting style (grouping separator, decimal separator).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum NumberFormat {
    /// `1 234 567,89` — space grouping, comma decimal
    #[default]
    SpaceComma = 0,
    /// `1,234,567.89` — comma grouping, dot decimal
    CommaDot = 1,
    /// `1.234.567,89` — dot grouping, comma decimal
    DotComma = 2,
    /// `1 234 567.89` — space grouping, dot decimal
    SpaceDot = 3,
}

/// Unit system for speed / distance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum UnitSystem {
    #[default]
    Metric = 0,
    Imperial = 1,
}

/// Temperature display unit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum TemperatureUnit {
    #[default]
    Celsius = 0,
    Fahrenheit = 1,
}

/// User formatting preferences passed from host to WASM widgets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormatPreferences {
    pub number_format: NumberFormat,
    pub unit_system: UnitSystem,
    pub temperature_unit: TemperatureUnit,
}

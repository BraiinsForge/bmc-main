// Copyright (C) 2026  Braiins Systems s.r.o.

//! Gauge color palette and the per-state lit ring fill, shared by the
//! mining-info gauge and both mining-clock rings. Render-only extras (the
//! mining-info status-label color and center glow) are composed by the caller
//! from these palette consts.

use bmc_wasm_sdk::{ArcFill, Color};

use crate::gauge::GaugeState;

pub const INACTIVE_TICK: Color = Color::from_rgb(0x1e, 0x1e, 0x1e);
pub const OFF_TICK: Color = Color::from_rgb(0xd9, 0x22, 0x2c);
pub const OFF_LABEL: Color = Color::from_rgb(0xf9, 0x53, 0x55);
pub const AMBER_DARK: Color = Color::from_rgb(0xcf, 0x79, 0x0e);
pub const AMBER_BRIGHT: Color = Color::from_rgb(0xfe, 0xba, 0x53);
pub const AMBER_LABEL: Color = Color::from_rgb(0xfe, 0xba, 0x53);
pub const GREEN_DARK: Color = Color::from_rgb(0x19, 0x5e, 0x33);
pub const GREEN_BRIGHT: Color = Color::from_rgb(0x5a, 0xdf, 0x88);
pub const GREEN_LABEL: Color = Color::from_rgb(0x34, 0xc0, 0x6a);
pub const PURPLE: Color = Color::from_rgb(0x8b, 0x7c, 0xff);

// The lit ring fill for each state, used by the mining-info gauge and both
// mining-clock rings. `None` for `NotAvailable`, which renders neutral.
#[must_use]
pub const fn ring_fill(state: GaugeState) -> Option<ArcFill> {
    match state {
        GaugeState::NotAvailable => None,
        GaugeState::Off => Some(ArcFill::Solid(OFF_TICK)),
        GaugeState::Underclocked => Some(ArcFill::gradient(AMBER_DARK, AMBER_BRIGHT)),
        GaugeState::Good => Some(ArcFill::gradient(GREEN_DARK, GREEN_BRIGHT)),
        GaugeState::Overclocked => Some(ArcFill::Solid(PURPLE)),
    }
}

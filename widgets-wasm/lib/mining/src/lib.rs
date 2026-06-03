// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared mining-gauge model and color palette for the mining widgets.
//!
//! `gauge` holds the state classification and tick geometry (no SDK types, so
//! it unit-tests on the host); `style` holds the palette and the per-state ring
//! fill. Both `mining-info` and `mining-clock` build their gauges from here.

pub mod gauge;
pub mod overlay;
pub mod style;

// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED peripheral control guest imports.

#![expect(clippy::cast_possible_truncation)]

use anyhow::Result;
use wasmi::{Caller, Linker};

use crate::host_api::{FixtureEvent, FixtureEventKind, HostState};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_set_effect_import(linker)?;
    register_set_brightness_import(linker)?;
    register_enable_import(linker)?;
    register_disable_import(linker)?;
    Ok(())
}

fn register_set_effect_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_led_set_effect",
        |mut caller: Caller<'_, HostState>,
         effect: u32,
         r: u32,
         g: u32,
         b: u32,
         period_ms: u32,
         duration_ms: u32| {
            let color = bmc_led::data::Rgb::new(r as u8, g as u8, b as u8);
            let kind = match bmc_led::data::LedEffectKind::try_from(effect as u8) {
                Ok(kind) => kind,
                Err(other) => {
                    tracing::warn!("ignoring unknown LED effect discriminant: {other}");
                    return;
                }
            };
            let led_effect = match kind {
                bmc_led::data::LedEffectKind::None => bmc_led::data::LedEffect::None,
                bmc_led::data::LedEffectKind::Chase => bmc_led::data::LedEffect::Chase(color),
                bmc_led::data::LedEffectKind::KnightRider => {
                    bmc_led::data::LedEffect::KnightRider(color)
                }
                bmc_led::data::LedEffectKind::Scan => bmc_led::data::LedEffect::Scan(color),
                bmc_led::data::LedEffectKind::Snake => bmc_led::data::LedEffect::Snake(color),
                bmc_led::data::LedEffectKind::Breathe => bmc_led::data::LedEffect::Breathe(color),
                bmc_led::data::LedEffectKind::Solid => bmc_led::data::LedEffect::Solid(color),
            };

            let state = caller.data_mut();
            let kind = FixtureEventKind::LedSetEffect {
                effect: effect as u8,
                r: r as u8,
                g: g as u8,
                b: b as u8,
                period_ms,
                duration_ms,
            };

            if state.record_events && state.recorded_events.last().is_none_or(|e| e.kind != kind) {
                state.recorded_events.push(FixtureEvent {
                    at_ms: state.monotonic_ms,
                    kind,
                });
            }

            let Some(ref tx) = state.led_command_sender else {
                return;
            };

            let period =
                (period_ms > 0).then(|| std::time::Duration::from_millis(u64::from(period_ms)));
            let duration =
                (duration_ms > 0).then(|| std::time::Duration::from_millis(u64::from(duration_ms)));

            let scene = bmc_led::data::LedScene {
                effect: led_effect,
                period,
                duration,
            };
            let _ = tx.send(bmc_led::data::LedCommand::SetEffect(scene));
        },
    )?;
    Ok(())
}

fn register_set_brightness_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_led_set_brightness",
        |mut caller: Caller<'_, HostState>, brightness_bits: u32| {
            let raw = f32::from_bits(brightness_bits);
            if !raw.is_finite() {
                tracing::warn!("ignoring non-finite LED brightness: {raw}");
                return;
            }
            let brightness = raw.clamp(0.0, 1.0);

            let state = caller.data_mut();
            let kind = FixtureEventKind::LedSetBrightness { brightness };

            if state.record_events && state.recorded_events.last().is_none_or(|e| e.kind != kind) {
                state.recorded_events.push(FixtureEvent {
                    at_ms: state.monotonic_ms,
                    kind,
                });
            }

            if let Some(ref tx) = state.led_command_sender {
                let _ = tx.send(bmc_led::data::LedCommand::SetBrightness(brightness));
            }
        },
    )?;
    Ok(())
}

fn register_enable_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_led_enable",
        |mut caller: Caller<'_, HostState>| {
            let state = caller.data_mut();

            if state.record_events
                && state
                    .recorded_events
                    .last()
                    .is_none_or(|e| e.kind != FixtureEventKind::LedEnable)
            {
                state.recorded_events.push(FixtureEvent {
                    at_ms: state.monotonic_ms,
                    kind: FixtureEventKind::LedEnable,
                });
            }

            if let Some(ref tx) = state.led_command_sender {
                let _ = tx.send(bmc_led::data::LedCommand::Enable);
            }
        },
    )?;
    Ok(())
}

fn register_disable_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_led_disable",
        |mut caller: Caller<'_, HostState>| {
            let state = caller.data_mut();

            if state.record_events
                && state
                    .recorded_events
                    .last()
                    .is_none_or(|e| e.kind != FixtureEventKind::LedDisable)
            {
                state.recorded_events.push(FixtureEvent {
                    at_ms: state.monotonic_ms,
                    kind: FixtureEventKind::LedDisable,
                });
            }

            if let Some(ref tx) = state.led_command_sender {
                let _ = tx.send(bmc_led::data::LedCommand::Disable);
            }
        },
    )?;
    Ok(())
}

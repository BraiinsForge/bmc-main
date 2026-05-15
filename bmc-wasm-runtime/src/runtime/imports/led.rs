// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED peripheral control guest imports.

#![expect(clippy::cast_possible_truncation)]

use std::time::Duration;

use anyhow::Result;
use wasmi::{Caller, Linker};

use bmc_led::data::LedEffectKind as LedEffect;

use crate::host_api::{FixtureEvent, FixtureEventKind, HostState};
use crate::led_request::{LED_REQUEST_ID_ALL, LedRequest};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_set_endless_import(linker)?;
    register_set_temporary_import(linker)?;
    register_stop_import(linker)?;
    Ok(())
}

fn register_set_endless_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_led_set_endless",
        |mut caller: Caller<'_, HostState>, effect: u32, r: u32, g: u32, b: u32, period_ms: u32| {
            emit_set(&mut caller, effect, r, g, b, period_ms, None);
        },
    )?;
    Ok(())
}

fn register_set_temporary_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_led_set_temporary",
        |mut caller: Caller<'_, HostState>,
         effect: u32,
         r: u32,
         g: u32,
         b: u32,
         period_ms: u32,
         duration_ms: u32| {
            emit_set(&mut caller, effect, r, g, b, period_ms, Some(duration_ms));
        },
    )?;
    Ok(())
}

fn register_stop_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_led_stop",
        |mut caller: Caller<'_, HostState>| {
            let state = caller.data_mut();
            let kind = FixtureEventKind::LedStop;
            if state.record_events && state.recorded_events.last().is_none_or(|e| e.kind != kind) {
                state.recorded_events.push(FixtureEvent {
                    at_ms: state.monotonic_ms,
                    kind,
                });
            }
            if let Some(ref tx) = state.led_request_sender {
                let _ = tx.send(LedRequest::Stop {
                    request_id: LED_REQUEST_ID_ALL,
                });
            }
        },
    )?;
    Ok(())
}

fn emit_set(
    caller: &mut Caller<'_, HostState>,
    effect: u32,
    r: u32,
    g: u32,
    b: u32,
    period_ms: u32,
    duration_ms: Option<u32>,
) {
    let Ok(effect) = LedEffect::try_from(effect as u8) else {
        tracing::warn!("ignoring unknown LED effect discriminant: {effect}");
        return;
    };
    let color = bmc_led::data::Rgb::new(r as u8, g as u8, b as u8);
    let state = caller.data_mut();

    let kind = match duration_ms {
        None => FixtureEventKind::LedSetEndless {
            effect: effect as u8,
            r: r as u8,
            g: g as u8,
            b: b as u8,
            period_ms,
        },
        Some(d) => FixtureEventKind::LedSetTemporary {
            effect: effect as u8,
            r: r as u8,
            g: g as u8,
            b: b as u8,
            period_ms,
            duration_ms: d,
        },
    };
    if state.record_events && state.recorded_events.last().is_none_or(|e| e.kind != kind) {
        state.recorded_events.push(FixtureEvent {
            at_ms: state.monotonic_ms,
            kind,
        });
    }

    let request_id = state.led_request_alloc.alloc();
    let Some(ref tx) = state.led_request_sender else {
        return;
    };
    let _ = tx.send(LedRequest::SetEffect {
        request_id,
        effect,
        color,
        period_ms,
        duration: duration_ms.map(|n| Duration::from_millis(u64::from(n))),
    });
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED peripheral control guest imports.

use std::time::Duration;

use anyhow::Result;
use wasmi::{Caller, Linker};

use bmc_led::data::{LedEffectKind as LedEffect, LedScope};

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
        |mut caller: Caller<'_, HostState>,
         effect: u32,
         r: u32,
         g: u32,
         b: u32,
         period_ms: u32,
         scope: u32| {
            emit_set(&mut caller, effect, r, g, b, period_ms, None, scope);
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
         duration_ms: u32,
         scope: u32| {
            emit_set(
                &mut caller,
                effect,
                r,
                g,
                b,
                period_ms,
                Some(duration_ms),
                scope,
            );
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

#[expect(
    clippy::too_many_arguments,
    reason = "parameters mirror the wire signature; bundling them into a struct adds ceremony without clarifying"
)]
fn emit_set(
    caller: &mut Caller<'_, HostState>,
    effect: u32,
    r: u32,
    g: u32,
    b: u32,
    period_ms: u32,
    duration_ms: Option<u32>,
    scope: u32,
) {
    let Ok(effect_byte) = u8::try_from(effect) else {
        tracing::warn!("ignoring out-of-range LED effect discriminant: {effect}");
        return;
    };
    let Ok(effect) = LedEffect::try_from(effect_byte) else {
        tracing::warn!("ignoring unknown LED effect discriminant: {effect_byte}");
        return;
    };
    let Ok(scope_byte) = u8::try_from(scope) else {
        tracing::warn!("ignoring out-of-range LED scope discriminant: {scope}");
        return;
    };
    let Ok(scope) = LedScope::try_from(scope_byte) else {
        tracing::warn!("ignoring unknown LED scope discriminant: {scope_byte}");
        return;
    };
    let (Ok(r), Ok(g), Ok(b)) = (u8::try_from(r), u8::try_from(g), u8::try_from(b)) else {
        tracing::warn!(
            r,
            g,
            b,
            "ignoring LED request with out-of-range RGB component"
        );
        return;
    };
    let color = bmc_led::data::Rgb::new(r, g, b);
    let state = caller.data_mut();

    let kind = match duration_ms {
        None => FixtureEventKind::LedSetEndless {
            effect: effect as u8,
            r,
            g,
            b,
            period_ms,
            scope: scope as u8,
        },
        Some(d) => FixtureEventKind::LedSetTemporary {
            effect: effect as u8,
            r,
            g,
            b,
            period_ms,
            duration_ms: d,
            scope: scope as u8,
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
        scope,
    });
}

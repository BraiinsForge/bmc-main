// Copyright (C) 2026  Braiins Systems s.r.o.

//! System-level guest imports such as wall-clock time and randomness.

#![expect(clippy::cast_possible_truncation)]

use anyhow::Result;
use chrono::{Datelike, Timelike};
use wasmi::{Caller, Extern, Linker};

use crate::host_api::HostState;

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_system_time_import(linker)?;
    register_random_import(linker)?;
    register_widget_size_import(linker)?;
    Ok(())
}

/// Widget viewport dimensions, packed as `(width << 32) | height` so the guest
/// reads them in a single register without an out-pointer dance. Set by the host
/// when constructing the runtime and never mutated thereafter — the SDK's
/// `widget_size()` reads this once.
fn register_widget_size_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_widget_size",
        |caller: Caller<'_, HostState>| -> u64 {
            let s = caller.data();
            (u64::from(s.widget_width) << 32) | u64::from(s.widget_height)
        },
    )?;
    Ok(())
}

fn register_system_time_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_get_system_time",
        |mut caller: Caller<'_, HostState>, out_ptr: u32| {
            let now = caller.data().system_time;
            let mut buf = [0_u8; 20];
            buf[0..8].copy_from_slice(&now.timestamp().to_le_bytes());
            buf[8..12].copy_from_slice(&now.offset().local_minus_utc().to_le_bytes());
            #[expect(clippy::cast_sign_loss)]
            let year = now.year() as u16;
            buf[12..14].copy_from_slice(&year.to_le_bytes());
            buf[14] = now.month() as u8;
            buf[15] = now.day() as u8;
            buf[16] = now.hour() as u8;
            buf[17] = now.minute() as u8;
            buf[18] = now.second() as u8;
            buf[19] = now.weekday().num_days_from_monday() as u8;

            let memory = caller.get_export("memory").and_then(Extern::into_memory);
            if let Some(memory) = memory {
                let data = memory.data_mut(&mut caller);
                let start = out_ptr as usize;
                if start + 20 <= data.len() {
                    data[start..start + 20].copy_from_slice(&buf);
                }
            }
        },
    )?;

    Ok(())
}

fn register_random_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_random_u32",
        |mut caller: Caller<'_, HostState>| -> u32 {
            let state = caller.data_mut();
            // Lazy time-derived auto-seed: only kicks in when no caller-provided
            // seed has been honoured yet. `monotonic_ms | 1` keeps the seed
            // non-zero so xorshift doesn't degenerate to all-zeros.
            let mut s = state.rng_state.unwrap_or(state.monotonic_ms | 1);
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            state.rng_state = Some(s);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "host_random_u32 intentionally returns the low 32 bits of the xorshift state"
            )]
            {
                s as u32
            }
        },
    )?;

    Ok(())
}

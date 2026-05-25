// Copyright (C) 2026  Braiins Systems s.r.o.

//! System-level guest imports: wall-clock time, randomness, widget viewport,
//! and the deck-wide `SystemSnapshot` snapshot channel (timezone, formatting preferences, next-alarm).

#![expect(clippy::cast_possible_truncation)]

use anyhow::Result;
use bmc_shared_time::time::Timezone;
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::OffsetComponents;
use wasmi::{Caller, Extern, Linker};

use super::super::memory::read_string;
use crate::host_api::HostState;

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_system_time_import(linker)?;
    register_random_import(linker)?;
    register_widget_size_import(linker)?;
    register_system_version(linker)?;
    register_system_snapshot(linker)?;
    register_resolve_tz_import(linker)?;
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
            let unix_secs = caller.data().system_time.timestamp();
            let buf = unix_secs.to_le_bytes();
            let memory = caller.get_export("memory").and_then(Extern::into_memory);
            if let Some(memory) = memory {
                let data = memory.data_mut(&mut caller);
                let start = out_ptr as usize;
                if start + 8 <= data.len() {
                    data[start..start + 8].copy_from_slice(&buf);
                }
            }
        },
    )?;

    Ok(())
}

/// `host_system_version() -> u64` — opaque change marker
/// for the `SystemSnapshot` channel. The SDK compares it against
/// its last-seen value and re-fetches the snapshot whenever they differ.
///
/// Wrap-safe (different = changed, not greater = newer).
/// Parallel to `host_params_version` for the params channel.
fn register_system_version(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_system_version",
        |caller: Caller<'_, HostState>| -> u64 { caller.data().system.version() },
    )?;
    Ok(())
}

/// `host_system_snapshot(out_ptr: *mut u8, out_cap: u32) -> u32`
/// — probe-then-allocate fetch of the encoded `SystemSnapshot`.
///
/// `out_cap == 0` returns the required byte length without writing;
/// `out_cap >= required` writes the packed snapshot and returns the bytes written;
/// `out_cap < required` writes nothing and returns the required length.
///
/// Mirrors `host_params_snapshot` exactly — same OOB-trap contract
/// and same memory-export-required ABI rule.
fn register_system_snapshot(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_system_snapshot",
        |mut caller: Caller<'_, HostState>,
         out_ptr: u32,
         out_cap: u32|
         -> std::result::Result<u32, wasmi::Error> {
            let bytes = caller.data_mut().system.encoded().to_vec();
            let needed = bytes.len() as u32;

            if out_cap < needed {
                return Ok(needed);
            }

            let Some(memory) = caller.get_export("memory").and_then(Extern::into_memory) else {
                return Err(wasmi::Error::new(
                    "host import `host_system_snapshot`: guest module has no exported `memory` \
                     — cannot write the snapshot. ABI requires an exported linear memory.",
                ));
            };
            let data = memory.data_mut(&mut caller);
            let start = out_ptr as usize;
            let end = start.saturating_add(bytes.len());
            if end > data.len() {
                return Err(wasmi::Error::new(format!(
                    "host import `host_system_snapshot`: out_ptr range {start:#x}..{end:#x} \
                     overflows guest memory of {} bytes — caller must size the buffer using \
                     the probe call (out_cap == 0) before writing",
                    data.len(),
                )));
            }
            data[start..end].copy_from_slice(&bytes);
            Ok(needed)
        },
    )?;
    Ok(())
}

/// `host_resolve_tz(name_ptr: *const u8, name_len: u32, unix_secs: i64) -> i32`
/// — look up the UTC offset (in seconds) for an IANA timezone at a moment.
///
/// Validates the name against the deck's supported list (the same
/// `bmc_shared_time::timezone_variant::TIMEZONE_VARIANTS` that backs
/// `tz!`'s compile-time check, sourced from openwrt/LuCI's
/// `zoneinfo.uc`). Returns `i32::MIN` when the name is unknown
/// — real UTC offsets are bounded to ±14 hours, so the sentinel never collides.
fn register_resolve_tz_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_resolve_tz",
        |caller: Caller<'_, HostState>, name_ptr: u32, name_len: u32, unix_secs: i64| -> i32 {
            let Some(name) = read_string(&caller, name_ptr, name_len) else {
                return i32::MIN;
            };
            let Some(tz) = Timezone::lookup(&name) else {
                return i32::MIN;
            };
            let Some(dt) = DateTime::<Utc>::from_timestamp(unix_secs, 0) else {
                return i32::MIN;
            };
            // Evaluate the offset at the *requested* moment, not "now",
            // so DST transitions are respected when the caller asks about
            // a past/future time.
            let offset = tz.chrono().offset_from_utc_datetime(&dt.naive_utc());
            let total = offset.base_utc_offset() + offset.dst_offset();
            #[expect(
                clippy::cast_possible_truncation,
                reason = "UTC offsets are bounded to ±14h ≈ ±50400s, fits in i32 with headroom"
            )]
            let secs = total.num_seconds() as i32;
            secs
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

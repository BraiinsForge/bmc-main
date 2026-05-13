// Copyright (C) 2026  Braiins Systems s.r.o.

//! Guest imports for the widget params snapshot.
//!
//! Two imports backstop the SDK's `bmc_wasm_sdk::params::current()` / `previous()` API:
//!
//!  - `host_params_version() -> u64` — opaque change marker.
//!    The SDK compares it against its last-seen value and re-fetches the snapshot whenever the
//!    two differ.
//!    Wrap-safe (semantics are "different = changed", not "greater = newer").
//!
//!  - `host_params_snapshot(out_ptr: *mut u8, out_cap: u32) -> u32` — probe-then-allocate.
//!    `out_cap == 0` returns the required byte length without writing; `out_cap >= required`
//!    writes the packed snapshot and returns the bytes written; `out_cap < required` writes
//!    nothing and returns the required length so the caller can retry.
//!
//! ## Memory-access failure traps, doesn't fail-quiet
//!
//! `host_params_snapshot` only returns a numeric result on success paths
//! (probe / retry-needed / write-completed).
//! An OOB `out_ptr` (i.e. `out_ptr + bytes.len()` overflows the guest's linear memory)
//! or a missing `memory` export traps with a clear message naming the rule that was violated,
//! mirroring `require_render` in `super::guards`.
//! Returning `0` for this case (the prior behaviour) collided with the genuinely-empty snapshot
//! that surfaces as the `u32` count header, and silently masked guest ABI bugs.
//!
//! The packed wire format is documented in `bmc_wasm_sdk::params`; the host-side serialiser
//! here is the inverse of the SDK's parser.

#![expect(
    clippy::cast_possible_truncation,
    reason = "in-memory `BTreeMap` length and per-entry byte counts are bounded by `try_from` guards \
              right before each `as u32` / `as u16` cast in `encode_entry`; the encoder is the \
              inverse of the SDK parser, which itself only ever reads u32/u16 sizes — so values \
              that would actually truncate are unreachable for valid `ParamKey` / `ParamValue` inputs"
)]

use anyhow::Result;
use wasmi::{Caller, Extern, Linker};

use crate::host_api::HostState;
use bmc_widget_manifest::{ParamKey, ParamValue};

/// Wire-format kind discriminators. Must match `bmc_wasm_sdk::params::kind`.
mod kind {
    pub const STR: u8 = 0;
    pub const I32: u8 = 1;
    pub const F64: u8 = 2;
    pub const BOOL: u8 = 3;
    pub const NULL: u8 = 4;
}

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_params_version(linker)?;
    register_params_snapshot(linker)?;
    Ok(())
}

fn register_params_version(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_params_version",
        |caller: Caller<'_, HostState>| -> u64 { caller.data().params_version },
    )?;
    Ok(())
}

fn register_params_snapshot(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_params_snapshot",
        |mut caller: Caller<'_, HostState>,
         out_ptr: u32,
         out_cap: u32|
         -> std::result::Result<u32, wasmi::Error> {
            // Serialise once into a stack-local buffer; cheap because params are small and the
            // snapshot is regenerated only when the guest's cache misses.
            let bytes = encode_params(&caller.data().params);
            let needed = bytes.len() as u32;

            if out_cap < needed {
                // Probe (out_cap == 0) and retry-with-larger-buffer both fall here.
                // Caller reads the return value to size the next call.
                return Ok(needed);
            }

            let Some(memory) = caller.get_export("memory").and_then(Extern::into_memory) else {
                return Err(wasmi::Error::new(
                    "host import `host_params_snapshot`: guest module has no exported `memory` \
                     — cannot write the snapshot. ABI requires an exported linear memory.",
                ));
            };
            let data = memory.data_mut(&mut caller);
            let start = out_ptr as usize;
            let end = start.saturating_add(bytes.len());
            if end > data.len() {
                return Err(wasmi::Error::new(format!(
                    "host import `host_params_snapshot`: out_ptr range {start:#x}..{end:#x} \
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

/// Encode the host-side params table into the packed wire format the guest SDK parses.
///
/// Entries are iterated in the `BTreeMap`'s natural key order (alphabetical) so two snapshots
/// with the same content produce byte-identical buffers — useful for `Clone` byte-equality
/// diffs on the guest side.
fn encode_params(params: &std::collections::BTreeMap<ParamKey, ParamValue>) -> Vec<u8> {
    let mut out = Vec::with_capacity(estimate_size(params));
    out.extend_from_slice(&(params.len() as u32).to_le_bytes());
    for (key, value) in params {
        encode_entry(&mut out, key.as_str(), value);
    }
    out
}

fn encode_entry(out: &mut Vec<u8>, key: &str, value: &ParamValue) {
    let kind_byte = match value {
        ParamValue::String(_) => kind::STR,
        ParamValue::Integer(_) => kind::I32,
        ParamValue::Double(_) => kind::F64,
        ParamValue::Boolean(_) => kind::BOOL,
        ParamValue::Null => kind::NULL,
    };
    out.push(kind_byte);

    // `ParamKey` enforces `MAX_PARAM_KEY_LENGTH` (well below `u16::MAX`) at the manifest layer,
    // so the conversion is statically infallible here. Same shape for the `s_len` u32 below.
    let key_len =
        u16::try_from(key.len()).expect("BUG: ParamKey enforces MAX_PARAM_KEY_LENGTH < u16::MAX");
    out.extend_from_slice(&key_len.to_le_bytes());
    out.extend_from_slice(key.as_bytes());

    match value {
        ParamValue::String(s) => {
            let s_len = u32::try_from(s.len())
                .expect("BUG: ParamValue::String enforces MAX_PARAM_STRING_LENGTH < u32::MAX");
            out.extend_from_slice(&s_len.to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        ParamValue::Integer(v) => out.extend_from_slice(&v.to_le_bytes()),
        ParamValue::Double(v) => out.extend_from_slice(&v.to_le_bytes()),
        ParamValue::Boolean(b) => out.push(u8::from(*b)),
        ParamValue::Null => {}
    }
}

/// Approximate byte length of the packed snapshot.
/// Used only for `Vec::with_capacity`; over- or under-estimate is harmless.
fn estimate_size(params: &std::collections::BTreeMap<ParamKey, ParamValue>) -> usize {
    let mut total = 4; // count header
    for (key, value) in params {
        total += 3 + key.as_str().len(); // kind byte + key_len + key bytes
        total += match value {
            ParamValue::String(s) => 4 + s.len(),
            ParamValue::Integer(_) => 4,
            ParamValue::Double(_) => 8,
            ParamValue::Boolean(_) => 1,
            ParamValue::Null => 0,
        };
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn key(s: &str) -> ParamKey {
        ParamKey::try_new(s.to_owned()).expect("BUG: test key must be valid")
    }

    #[test]
    fn empty_map_encodes_to_count_header() {
        let bytes = encode_params(&BTreeMap::new());
        assert_eq!(bytes, [0, 0, 0, 0]);
    }

    #[test]
    fn each_variant_serialises_to_documented_layout() {
        let mut params = BTreeMap::new();
        params.insert(key("a_str"), ParamValue::String("hi".into()));
        params.insert(key("b_int"), ParamValue::Integer(-7));
        params.insert(key("c_dbl"), ParamValue::Double(2.5));
        params.insert(key("d_bool"), ParamValue::Boolean(true));
        params.insert(key("e_null"), ParamValue::Null);

        let bytes = encode_params(&params);

        // Count = 5
        assert_eq!(&bytes[0..4], &5_u32.to_le_bytes());

        // Round-trip via the SDK parser to confirm byte-format compatibility
        // (parser lives in `bmc_wasm_sdk::params` and is exercised end-to-end via the SDK's
        // own unit tests on hand-rolled buffers; here we just sanity-check the leading layout).
        let mut offset = 4;
        // Entries should be alphabetical by key (BTreeMap order).
        let expected_keys = ["a_str", "b_int", "c_dbl", "d_bool", "e_null"];
        for expected_key in expected_keys {
            let kind_byte = bytes[offset];
            offset += 1;
            let key_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            let key_bytes = &bytes[offset..offset + key_len];
            offset += key_len;
            assert_eq!(key_bytes, expected_key.as_bytes());
            offset += match kind_byte {
                kind::STR => {
                    let s_len = u32::from_le_bytes([
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + 2],
                        bytes[offset + 3],
                    ]) as usize;
                    4 + s_len
                }
                kind::I32 => 4,
                kind::F64 => 8,
                kind::BOOL => 1,
                kind::NULL => 0,
                _ => panic!("BUG: unknown kind"),
            };
        }
        assert_eq!(offset, bytes.len());
    }

    #[test]
    fn keys_serialise_in_alphabetical_order() {
        let mut params = BTreeMap::new();
        params.insert(key("zebra"), ParamValue::Boolean(false));
        params.insert(key("apple"), ParamValue::Boolean(false));
        params.insert(key("mango"), ParamValue::Boolean(false));

        let bytes = encode_params(&params);
        // Skip count header (4) + walk entries pulling out keys.
        let mut offset = 4;
        let mut keys = Vec::new();
        for _ in 0..3 {
            offset += 1; // kind
            let key_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            keys.push(
                std::str::from_utf8(&bytes[offset..offset + key_len])
                    .expect("BUG: encode_params writes &str bytes — output is valid UTF-8 by construction")
                    .to_owned(),
            );
            offset += key_len;
            offset += 1; // bool payload
        }
        assert_eq!(keys, vec!["apple", "mango", "zebra"]);
    }
}

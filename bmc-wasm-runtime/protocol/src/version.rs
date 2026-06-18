// Copyright (C) 2026  Braiins Systems s.r.o.

//! SDK protocol version for host/widget compatibility.
//!
//! Every widget exports `__bmc_sdk_init`; the host calls it once to read the
//! version (rejecting on major mismatch) and install the panic hook.

/// Current SDK protocol version (major, minor, patch).
///
/// Bump on any breaking change to:
/// - Host function signatures (names, parameter types, return types)
/// - Binary tree serialization format (node types, field layout)
/// - Host-side behavioral contracts (e.g. button click reporting)
pub const SDK_VERSION: (u16, u16, u16) = (0, 1, 0);

/// The instantiation-handshake export: returns the version, installs the hook.
pub const SDK_INIT_EXPORT: &str = "__bmc_sdk_init";

/// Pack a version tuple into a u64 for passing through WASM export.
#[must_use]
pub const fn version_pack(v: (u16, u16, u16)) -> u64 {
    (v.0 as u64) | ((v.1 as u64) << 16) | ((v.2 as u64) << 32)
}

/// Unpack a u64 from the WASM export into a version tuple.
#[must_use]
#[expect(clippy::cast_possible_truncation)]
pub const fn version_unpack(packed: u64) -> (u16, u16, u16) {
    (packed as u16, (packed >> 16) as u16, (packed >> 32) as u16)
}

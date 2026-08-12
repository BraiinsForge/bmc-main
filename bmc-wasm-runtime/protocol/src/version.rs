// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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
pub const SDK_VERSION: (u16, u16, u16) = (0, 6, 0);

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

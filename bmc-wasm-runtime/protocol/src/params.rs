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

//! Wire format for widget params snapshots.
//!
//! Shared between the host encoder (`bmc-wasm-runtime::runtime::imports::params`) and the
//! guest decoder (`bmc-wasm-sdk::params`). The byte layout itself is documented in
//! `bmc_wasm_sdk::params`; this module owns only the cross-side invariants
//! (kind discriminators) so the two implementations cannot drift on the constants.

/// Wire-format kind discriminators for `ParamValue` variants.
///
/// One byte per entry, preceding the variant-specific payload.
pub mod kind {
    pub const STR: u8 = 0;
    pub const I32: u8 = 1;
    pub const F64: u8 = 2;
    pub const BOOL: u8 = 3;
    pub const NULL: u8 = 4;
}

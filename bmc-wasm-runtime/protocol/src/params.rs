// Copyright (C) 2026  Braiins Systems s.r.o.

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

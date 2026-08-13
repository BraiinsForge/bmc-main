// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
include!(concat!(env!("OUT_DIR"), "/asset_record.rs"));

#[must_use]
pub fn descriptor_byte() -> u32 {
    #[cfg(target_arch = "wasm32")]
    {
        return ASSET_ID[0] as u32;
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

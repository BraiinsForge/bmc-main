// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
include!(concat!(env!("OUT_DIR"), "/asset_record.rs"));

#[cfg(target_arch = "wasm32")]
const _: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../widgets-wasm/spacex-launch/assets/falcon-9.png"
));

#[unsafe(no_mangle)]
pub extern "C" fn package_asset_descriptor() -> u32 {
    #[cfg(target_arch = "wasm32")]
    {
        return u32::from(ASSET_ID[0]) + bmc_wasm_assets_fixture_dep::descriptor_byte();
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

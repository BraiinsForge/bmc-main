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

/// Get a random `u32` from the host: seeded from the clock on the device,
/// while tools such as capture and the test host may pin the seed.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn random_u32() -> u32 {
    Platform::random_u32()
}

#[cfg(target_arch = "wasm32")]
trait RandomProvider {
    fn random_u32() -> u32;
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_random_u32() -> u32;
    }

    pub(super) struct Host;

    impl super::RandomProvider for Host {
        fn random_u32() -> u32 {
            unsafe { host_random_u32() }
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

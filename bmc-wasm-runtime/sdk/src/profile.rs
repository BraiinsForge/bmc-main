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

//! Fuel-based section profiling: a span charges the `wasmi` fuel (instruction
//! count, hardware-independent) spent in its scope to the host on drop. Gated by
//! the `profiling` feature — a zero-cost no-op `Span` otherwise.
//!
//! ```ignore
//! let _s = profile::span("ground_track");
//! ```

#[cfg(all(target_arch = "wasm32", feature = "profiling"))]
mod imp {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_fuel_remaining() -> u64;
        fn host_profile_section(name_ptr: *const u8, name_len: u32, fuel: u64);
    }

    /// Charges the fuel spent in its scope to `name` on drop.
    #[derive(Debug)]
    pub struct Span {
        name: &'static str,
        start_fuel: u64,
    }

    impl Drop for Span {
        fn drop(&mut self) {
            let used = self
                .start_fuel
                .saturating_sub(unsafe { host_fuel_remaining() });
            unsafe { host_profile_section(self.name.as_ptr(), self.name.len() as u32, used) };
        }
    }

    #[must_use]
    pub fn span(name: &'static str) -> Span {
        Span {
            name,
            start_fuel: unsafe { host_fuel_remaining() },
        }
    }
}

#[cfg(not(all(target_arch = "wasm32", feature = "profiling")))]
mod imp {
    /// No-op span guard when profiling is off.
    #[derive(Debug)]
    pub struct Span;

    #[must_use]
    pub fn span(_name: &'static str) -> Span {
        Span
    }
}

pub use imp::{Span, span};

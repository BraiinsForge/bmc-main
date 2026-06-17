// Copyright (C) 2026  Braiins Systems s.r.o.

//! Fuel-based section profiling: a span charges the `wasmi` fuel (instruction
//! count, hardware-independent) spent in its scope to the host on drop. Gated by
//! the `profiling` feature — a zero-cost no-op `Span` otherwise.
//!
//! ```ignore
//! let _s = profile::span("ground_track");
//! ```

#[cfg(all(target_arch = "wasm32", feature = "profiling"))]
mod imp {
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

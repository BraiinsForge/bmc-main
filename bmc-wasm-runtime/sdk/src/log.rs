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

//! Guest-side logging routed through host tracing.
//!
//! Messages are forwarded to the host via `host_log` and emitted through the
//! host's tracing subscriber at the requested level.
//!
//! # Example
//!
//! ```ignore
//! log_info!("position updated: lat={}, lon={}", lat, lon);
//! log_warn!("TLE data missing, orbit track unavailable");
//! log_error!("API returned status {}", status);
//! ```

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_log(ptr: *const u8, len: u32, level: u32);
}

/// Log level constants matching tracing levels.
pub mod level {
    pub const DEBUG: u32 = 0;
    pub const INFO: u32 = 1;
    pub const WARN: u32 = 2;
    pub const ERROR: u32 = 3;
}

/// Send a log message to the host. Prefer the `log_*!` macros instead.
#[doc(hidden)]
pub fn _log(level: u32, msg: &str) {
    unsafe { host_log(msg.as_ptr(), msg.len() as u32, level) }
}

/// Log at debug level.
///
/// Uses the same formatting as [`fmt!`](crate::fmt).
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::log::_log($crate::log::level::DEBUG, &$crate::fmt!($($arg)*))
    };
}

/// Log at info level.
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::log::_log($crate::log::level::INFO, &$crate::fmt!($($arg)*))
    };
}

/// Log at warn level.
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::log::_log($crate::log::level::WARN, &$crate::fmt!($($arg)*))
    };
}

/// Log at error level.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::log::_log($crate::log::level::ERROR, &$crate::fmt!($($arg)*))
    };
}

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

//! The host's clock, on the shared [`SystemTime`].

pub use bmc_ui::time::SystemTime;
use bmc_wasm_protocol::time::LocalDateTime;

/// What only a host can do with a [`SystemTime`]: read
/// the clock and project the instant into a time zone.
/// An extension trait, since the type is `bmc-ui`'s and calls no host.
pub trait HostClock: Sized {
    #[must_use]
    fn now() -> Self;

    /// Project into local wall-clock fields for `tz`.
    /// Returns `None` when the host doesn't recognise the tz name.
    #[must_use]
    fn local(&self, tz: &crate::Tz) -> Option<LocalDateTime>;

    #[must_use]
    fn utc(&self) -> LocalDateTime;
}

impl HostClock for SystemTime {
    fn now() -> Self {
        Self {
            unix_secs: Platform::unix_secs(),
        }
    }

    fn local(&self, tz: &crate::Tz) -> Option<LocalDateTime> {
        crate::calendar::tz_convert(self.unix_secs, tz.iana())
    }

    fn utc(&self) -> LocalDateTime {
        decompose(self.unix_secs)
    }
}

fn decompose(unix_secs: i64) -> LocalDateTime {
    fn parse<T: core::str::FromStr + Default>(p: Option<&str>) -> T {
        p.and_then(|s| s.parse().ok()).unwrap_or_default()
    }
    let s = crate::format::strftime(unix_secs, "%Y %m %d %H %M %S %u");
    let mut parts = s.split_ascii_whitespace();
    let year: u16 = parse(parts.next());
    let month: u8 = parse(parts.next());
    let day: u8 = parse(parts.next());
    let hour: u8 = parse(parts.next());
    let minute: u8 = parse(parts.next());
    let second: u8 = parse(parts.next());
    // `%u` is ISO weekday: 1=Mon, 7=Sun. Re-map to 0=Mon, 6=Sun.
    let iso_weekday: u8 = parse(parts.next());
    let weekday = iso_weekday.saturating_sub(1);
    LocalDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday,
    }
}

trait ClockProvider {
    fn unix_secs() -> i64;
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_get_system_time(out_ptr: *mut u8);
    }

    pub(super) struct Host;

    impl super::ClockProvider for Host {
        fn unix_secs() -> i64 {
            let mut buf = [0_u8; 8];
            unsafe { host_get_system_time(buf.as_mut_ptr()) }
            i64::from_le_bytes(buf)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    pub(super) struct Host;

    impl super::ClockProvider for Host {
        fn unix_secs() -> i64 {
            panic!(
                "BUG: SystemTime::now() called off-target — no host keeps the time; supply it explicitly"
            )
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;

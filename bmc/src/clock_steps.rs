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

use tokio::sync::watch;

#[cfg(target_os = "linux")]
mod step_timer;

/// Counts `CLOCK_REALTIME` steps (NTP, `date -s`) in either direction, starting at zero.
///
/// The timer is armed before this returns,
/// so a step during the caller's own startup is recorded by the kernel instead of lost.
/// The receiver closes if the timer fails. Must be called from within a Tokio runtime.
#[cfg(target_os = "linux")]
pub(crate) fn watch_clock_steps() -> watch::Receiver<u64> {
    step_timer::watch().unwrap_or_else(|err| {
        tracing::error!(error = %err, "Cannot watch for wall-clock steps");
        closed()
    })
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn watch_clock_steps() -> watch::Receiver<u64> {
    tracing::info!("No timerfd on this platform, wall-clock steps go unnoticed");
    closed()
}

fn closed() -> watch::Receiver<u64> {
    watch::channel(0).1
}

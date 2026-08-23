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

//! Time-bounded Wayland event dispatch.
//!
//! `EventQueue::blocking_dispatch` has no timeout, so any code that waits
//! for a specific event with a deadline (e.g. the configure-batch wait at
//! widget startup) needs the standard `wayland-rs`
//! prepare_read → poll → read | cancel → dispatch_pending recipe to make
//! the deadline actually enforceable. This module provides that helper
//! once for the GPU surface clients under `crate::surface`.

use std::os::fd::{AsFd, AsRawFd};

use anyhow::{Context, Result};
use bmc_widget_protocol::wayland_client::{Connection, EventQueue};

/// Outcome of a [`poll_dispatch`] call.
///
/// Distinguishes between `poll(2)` returning because an event arrived vs
/// the specified timeout expiring. Callers that drive their render loop
/// off a timer (e.g. rendering at the next wall-clock-second boundary)
/// need to distinguish these — a non-callback event wake (`wl_buffer.release`,
/// output reconfigure, etc.) must not be mistaken for a timeout expiry or
/// the loop feedbacks into busy-rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollOutcome {
    /// Events arrived and were dispatched (or events were already queued).
    /// Also covers benign `EAGAIN`/`EWOULDBLOCK` races where pending events
    /// were still dispatched.
    Events,
    /// `poll(2)` returned 0 — the requested timeout expired without any
    /// event arriving on the Wayland fd.
    Timeout,
}

/// Poll for Wayland events with a timeout, then dispatch pending events.
///
/// Returns a [`PollOutcome`] distinguishing event vs timeout vs EAGAIN
/// so callers can tell a real timeout expiry from an event-driven wake.
/// A timeout of `-1` blocks indefinitely; `0` is non-blocking.
///
/// This follows the `prepare_read -> poll -> read | cancel -> dispatch_pending`
/// pattern required by `wayland-client`.
pub fn poll_dispatch<S: 'static>(
    conn: &Connection,
    queue: &mut EventQueue<S>,
    state: &mut S,
    timeout_ms: i32,
) -> Result<PollOutcome> {
    conn.flush()?;

    let read_guard = queue.prepare_read();
    let mut outcome = PollOutcome::Events;

    match read_guard {
        None => {
            // Events already queued -- just dispatch them
            queue
                .dispatch_pending(state)
                .context("Wayland dispatch failed")?;
            return Ok(PollOutcome::Events);
        }
        Some(guard) => {
            let fd = conn.as_fd();
            let mut pollfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };

            let poll_ret = unsafe { libc::poll(&raw mut pollfd, 1, timeout_ms) };

            match poll_ret.cmp(&0) {
                std::cmp::Ordering::Greater => match guard.read() {
                    Ok(_) => {}
                    Err(bmc_widget_protocol::wayland_client::backend::WaylandError::Io(err))
                        if err.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // Non-fatal race: poll reported readability but no
                        // event was available by the time we read. Fall
                        // through to dispatch_pending.
                    }
                    Err(err) => return Err(err).context("Wayland socket read failed"),
                },
                std::cmp::Ordering::Equal => {
                    // Timeout -- cancel read
                    drop(guard);
                    outcome = PollOutcome::Timeout;
                }
                std::cmp::Ordering::Less => {
                    // Error
                    let err = std::io::Error::last_os_error();
                    drop(guard);
                    #[expect(
                        clippy::wildcard_enum_match_arm,
                        reason = "all other io::ErrorKind variants are fatal"
                    )]
                    match err.kind() {
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock => {
                            // EINTR / EAGAIN -- not fatal, just dispatch pending
                        }
                        _ => {
                            return Err(err).context("poll(2) on Wayland fd failed");
                        }
                    }
                }
            }
        }
    }

    queue
        .dispatch_pending(state)
        .context("Wayland dispatch failed")?;

    Ok(outcome)
}

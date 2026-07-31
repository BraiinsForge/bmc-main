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

//! How long each stage of a shutdown may take.
//!
//! procd sends SIGTERM and then SIGKILL.
//! Every stage below runs inside that window, one after the other.
//! Only their sum matters — raising one steals from the rest,
//! and overshooting gets the process killed mid-stage.

use std::time::Duration;

/// procd's `term_timeout` — SIGTERM to SIGKILL.
/// Mirrors `nix/service.nix`, which sets one value for every service.
pub const TERM_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a sysupgrade keeps the web server up after SIGTERM,
/// so clients still receive the last progress events.
pub const UPGRADE_HOLD: Duration = Duration::from_secs(5);

/// How long a connection drain may run before it is worth mentioning.
/// An exit where every connection closes at once stays quiet.
pub const DRAIN_QUIET: Duration = Duration::from_secs(1);

/// When to stop draining and exit with connections still open.
pub const DRAIN_DEADLINE: Duration = Duration::from_secs(10);

const _: () = assert!(
    UPGRADE_HOLD.as_secs() + DRAIN_DEADLINE.as_secs() < TERM_TIMEOUT.as_secs(),
    "a sysupgrade shutdown holds the server open and then drains, \
     so both must finish before procd resorts to SIGKILL"
);

const _: () = assert!(
    DRAIN_QUIET.as_secs() < DRAIN_DEADLINE.as_secs(),
    "the drain warns first and gives up second"
);

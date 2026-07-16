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

//! Pacing of the simulated upgrade flows: realistic delays so a developer
//! sees progress unfold, or no delays so the test suite iterates the
//! states immediately.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradePacing {
    Realistic,
    Instant,
}

impl UpgradePacing {
    #[must_use]
    pub fn blob_chunk_delay(self) -> Duration {
        match self {
            Self::Realistic => Duration::from_millis(100),
            Self::Instant => Duration::ZERO,
        }
    }

    #[must_use]
    pub fn progress_step(self) -> Duration {
        match self {
            Self::Realistic => Duration::from_millis(300),
            Self::Instant => Duration::ZERO,
        }
    }

    #[must_use]
    pub fn sysupgrade_duration(self) -> Duration {
        match self {
            Self::Realistic => Duration::from_secs(10),
            Self::Instant => Duration::ZERO,
        }
    }

    // Instant keeps a small delay so the last progress event reaches the
    // client before the simulated reboot or application stop tears the
    // process down.
    #[must_use]
    pub fn shutdown_delay(self) -> Duration {
        match self {
            Self::Realistic => Duration::from_secs(2),
            Self::Instant => Duration::from_millis(200),
        }
    }
}

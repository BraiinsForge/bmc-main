// Copyright (C) 2026  Braiins Systems s.r.o.

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

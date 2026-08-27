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

use std::time::Duration;

const DEFAULT_RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const RESTART_BACKOFF_FACTOR: u32 = 2;
/// A ceiling, never a give-up.
/// A retry budget could permanently remove a process after a transient fault.
const DEFAULT_RESTART_BACKOFF_MAX: Duration = Duration::from_mins(5);
/// Do not reset the ladder merely because a process survived its initial startup.
const DEFAULT_RESTART_HEALTHY_UPTIME: Duration = Duration::from_mins(1);
const _: () = assert!(
    RESTART_BACKOFF_FACTOR >= 2,
    "a factor of 1 pins the ladder at its initial delay and 0 respawns without waiting at all"
);

/// Bounded restart delays for supervised processes.
#[derive(Debug, Clone, Copy)]
pub struct RestartPolicy {
    initial: Duration,
    max: Duration,
    healthy_uptime: Duration,
}

impl RestartPolicy {
    #[must_use]
    pub const fn new(initial: Duration, max: Duration, healthy_uptime: Duration) -> Self {
        Self {
            initial,
            max,
            healthy_uptime,
        }
    }

    #[must_use]
    pub const fn initial(self) -> Duration {
        self.initial
    }

    #[must_use]
    pub const fn max(self) -> Duration {
        self.max
    }

    #[must_use]
    pub const fn healthy_uptime(self) -> Duration {
        self.healthy_uptime
    }

    #[must_use]
    pub fn next_backoff(self, delay: Duration) -> Duration {
        (delay * RESTART_BACKOFF_FACTOR).min(self.max)
    }

    #[must_use]
    pub fn restart_delay(self, uptime: Duration, backoff: Duration) -> Duration {
        // Keep this threshold absolute rather than proportional to the current backoff.
        // Escalating a mostly healthy process would turn a brief failure into a long outage.
        if uptime >= self.healthy_uptime {
            self.initial
        } else {
            backoff
        }
    }
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self::new(
            DEFAULT_RESTART_BACKOFF_INITIAL,
            DEFAULT_RESTART_BACKOFF_MAX,
            DEFAULT_RESTART_HEALTHY_UPTIME,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RestartPolicy;

    #[test]
    fn backoff_doubles_up_to_the_ceiling() {
        let max = Duration::from_mins(5);
        let default = RestartPolicy::default();
        let policy = RestartPolicy::new(default.initial(), max, default.healthy_uptime());
        assert_eq!(
            policy.next_backoff(Duration::from_secs(1)),
            Duration::from_secs(2)
        );
        assert_eq!(
            policy.next_backoff(Duration::from_secs(128)),
            Duration::from_secs(256)
        );
        assert_eq!(
            policy.next_backoff(Duration::from_secs(256)),
            max,
            "doubling past the ceiling clamps to it"
        );
        assert_eq!(
            policy.next_backoff(max),
            max,
            "the ceiling is a fixed point"
        );
    }

    #[test]
    fn healthy_uptime_restarts_the_ladder() {
        let policy = RestartPolicy::default();
        let climbed = Duration::from_secs(256);

        assert_eq!(
            policy.restart_delay(policy.healthy_uptime(), climbed),
            policy.initial(),
            "reaching the healthy uptime exactly restarts the ladder"
        );
        assert_eq!(
            policy.restart_delay(Duration::ZERO, climbed),
            climbed,
            "a process that died on startup continues the ladder"
        );
    }
}

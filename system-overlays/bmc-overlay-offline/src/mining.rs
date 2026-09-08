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

//! The mining-status rule: what one poll of the BOS API says about the miner,
//! and how a run of failed polls is folded into the last known answer.
//!
//! Only the tuner state and the hashboards are read, which is why a stopped
//! miner shows as red only once its 1-minute and 5-minute hashrates have
//! drained, about a minute later.

/// An active board hashing under this share of its nominal is underperforming.
pub const UNDERPERFORMANCE_RATIO: f64 = 0.8;

/// Consecutive failed polls tolerated before the indicator turns red:
/// 25 s of silence at [`crate::poller::POLL_PERIOD`].
pub const FAILURES_BEFORE_LOW: usize = 5;

/// BOS `TunerState`, as `overall_tuner_state` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunerState {
    Disabled,
    Stable,
    Tuning,
    Error,
    Continuous,
    Preheat,
    /// A value this build does not know; never a tuning stage.
    Unknown(i32),
}

impl TunerState {
    /// Whether the tuner is still moving frequencies. A paused bosminer keeps
    /// reporting the stage it was interrupted in, so this alone does not mean
    /// the miner is tuning; the boards decide.
    #[must_use]
    pub fn is_tuning_stage(self) -> bool {
        match self {
            Self::Tuning | Self::Continuous | Self::Preheat => true,
            Self::Disabled | Self::Stable | Self::Error | Self::Unknown(_) => false,
        }
    }
}

/// One hashboard's readings, in GH/s.
/// `None` is a null on the wire, or a field the firmware left out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Board {
    pub enabled: bool,
    /// The board's theoretical rate at the clocks currently set.
    /// An unclocked board arrives as `Some(0.0)`, a board that reports no
    /// nominal at all as `None`; neither counts as active.
    pub nominal_ghs: Option<f64>,
    pub last_1m_ghs: Option<f64>,
    pub last_5m_ghs: Option<f64>,
}

impl Board {
    /// Enabled with a nominal to compare against. A board the user disabled
    /// is ignored on purpose, so one left off never holds the corner red.
    fn active(&self) -> bool {
        self.enabled && self.nominal_ghs.is_some_and(|nominal| nominal > 0.0)
    }

    fn hashing(&self) -> bool {
        self.active() && self.last_1m_ghs.is_some_and(|rate| rate > 0.0)
    }

    fn underperforming(&self) -> bool {
        if !self.active() {
            return false;
        }
        let nominal = self
            .nominal_ghs
            .expect("BUG: an active board has a nominal by definition");
        self.last_5m_ghs
            .is_none_or(|rate| rate < UNDERPERFORMANCE_RATIO * nominal)
    }
}

/// What one successful poll returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Readings {
    pub tuner: TunerState,
    pub boards: Vec<Board>,
}

/// The indicator's three answers. `Ok` draws nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Tuning,
    Ok,
    Low,
}

/// The mining status one poll's readings describe.
#[must_use]
pub fn derive(readings: &Readings) -> Status {
    let boards = &readings.boards;
    if readings.tuner.is_tuning_stage() {
        return if boards.iter().any(Board::hashing) {
            Status::Tuning
        } else {
            Status::Low
        };
    }
    let any_active = boards.iter().any(Board::active);
    let any_underperforming = boards.iter().any(Board::underperforming);
    if any_active && !any_underperforming {
        Status::Ok
    } else {
        Status::Low
    }
}

/// Folds a run of polls into the status to show:
/// the last successful answer survives up to [`FAILURES_BEFORE_LOW`]
/// consecutive failures, then gives way to `Low`.
/// Before the first success there is nothing to hold on to,
/// so the indicator stays hidden until the budget runs out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusTracker {
    shown: Option<Status>,
    consecutive_failures: usize,
}

/// The edges of a run of failures, as one observed poll crosses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Unchanged,
    /// This failure was the one that used up the budget.
    Exhausted,
    /// This success ended an exhausted run.
    Recovered,
}

impl StatusTracker {
    /// Record one poll: `Some` is the status a successful poll derived, `None`
    /// is a failed poll of any kind.
    pub fn observe(&mut self, derived: Option<Status>) -> Transition {
        if let Some(status) = derived {
            let recovered = self.exhausted();
            self.consecutive_failures = 0;
            self.shown = Some(status);
            if recovered {
                Transition::Recovered
            } else {
                Transition::Unchanged
            }
        } else {
            self.consecutive_failures += 1;
            if self.exhausted() {
                self.shown = Some(Status::Low);
            }
            if self.consecutive_failures == FAILURES_BEFORE_LOW {
                Transition::Exhausted
            } else {
                Transition::Unchanged
            }
        }
    }

    fn exhausted(&self) -> bool {
        self.consecutive_failures >= FAILURES_BEFORE_LOW
    }

    #[must_use]
    pub fn shown(&self) -> Option<Status> {
        self.shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOMINAL: f64 = 500.0;

    fn board(
        enabled: bool,
        nominal: Option<f64>,
        last_1m: Option<f64>,
        last_5m: Option<f64>,
    ) -> Board {
        Board {
            enabled,
            nominal_ghs: nominal,
            last_1m_ghs: last_1m,
            last_5m_ghs: last_5m,
        }
    }

    /// An enabled board hashing at its nominal on both means.
    fn healthy() -> Board {
        board(true, Some(NOMINAL), Some(NOMINAL), Some(NOMINAL))
    }

    fn readings(tuner: TunerState, boards: Vec<Board>) -> Readings {
        Readings { tuner, boards }
    }

    #[test]
    fn only_tuning_continuous_and_preheat_are_tuning_stages() {
        for state in [
            TunerState::Tuning,
            TunerState::Continuous,
            TunerState::Preheat,
        ] {
            assert!(state.is_tuning_stage(), "{state:?}");
        }
        for state in [
            TunerState::Disabled,
            TunerState::Stable,
            TunerState::Error,
            TunerState::Unknown(42),
        ] {
            assert!(!state.is_tuning_stage(), "{state:?}");
        }
    }

    #[test]
    fn a_tuning_stage_with_a_hashing_board_is_tuning() {
        for tuner in [
            TunerState::Tuning,
            TunerState::Continuous,
            TunerState::Preheat,
        ] {
            // Tuning boards run well under nominal; that is not underperformance yet.
            let slow = board(true, Some(NOMINAL), Some(50.0), Some(50.0));
            assert_eq!(
                derive(&readings(tuner, vec![slow])),
                Status::Tuning,
                "{tuner:?}"
            );
        }
    }

    #[test]
    fn a_tuning_stage_with_no_board_hashing_is_low() {
        // A paused miner: the tuner still reports its stage, the rates drain.
        let idle = board(true, Some(NOMINAL), Some(0.0), Some(120.0));
        assert_eq!(
            derive(&readings(TunerState::Tuning, vec![idle])),
            Status::Low
        );
        let unread = board(true, Some(NOMINAL), None, None);
        assert_eq!(
            derive(&readings(TunerState::Preheat, vec![unread])),
            Status::Low
        );
    }

    #[test]
    fn a_stable_miner_at_nominal_is_ok() {
        assert_eq!(
            derive(&readings(TunerState::Stable, vec![healthy(), healthy()])),
            Status::Ok
        );
        // A tuner switched off by config arrives as DISABLED and is judged the same way.
        assert_eq!(
            derive(&readings(TunerState::Disabled, vec![healthy()])),
            Status::Ok
        );
    }

    #[test]
    fn exactly_eighty_percent_of_nominal_is_still_ok() {
        let at_threshold = board(
            true,
            Some(NOMINAL),
            Some(NOMINAL),
            Some(NOMINAL * UNDERPERFORMANCE_RATIO),
        );
        assert_eq!(
            derive(&readings(TunerState::Stable, vec![at_threshold])),
            Status::Ok
        );
        let just_under = board(
            true,
            Some(NOMINAL),
            Some(NOMINAL),
            Some(NOMINAL * UNDERPERFORMANCE_RATIO - 0.5),
        );
        assert_eq!(
            derive(&readings(TunerState::Stable, vec![just_under])),
            Status::Low
        );
    }

    #[test]
    fn one_underperforming_board_among_healthy_ones_is_low() {
        let weak = board(true, Some(NOMINAL), Some(NOMINAL), Some(100.0));
        assert_eq!(
            derive(&readings(TunerState::Stable, vec![healthy(), weak])),
            Status::Low
        );
    }

    #[test]
    fn a_null_five_minute_mean_on_an_active_board_is_low() {
        let unread = board(true, Some(NOMINAL), Some(NOMINAL), None);
        assert_eq!(
            derive(&readings(TunerState::Stable, vec![unread])),
            Status::Low
        );
    }

    #[test]
    fn a_disabled_board_is_left_to_the_others() {
        let disabled = board(false, Some(NOMINAL), Some(0.0), Some(0.0));
        assert_eq!(
            derive(&readings(TunerState::Stable, vec![healthy(), disabled])),
            Status::Ok
        );
        assert_eq!(
            derive(&readings(TunerState::Stable, vec![disabled])),
            Status::Low,
            "alone, a disabled board leaves nothing active"
        );
    }

    #[test]
    fn a_board_without_a_nominal_is_not_active() {
        // A zero nominal is no rate to measure against, so the board counts
        // as inactive rather than as one performing perfectly.
        for nominal in [None, Some(0.0)] {
            let unclocked = board(true, nominal, Some(0.0), Some(0.0));
            assert_eq!(
                derive(&readings(TunerState::Stable, vec![unclocked])),
                Status::Low,
                "{nominal:?}"
            );
            assert_eq!(
                derive(&readings(TunerState::Stable, vec![healthy(), unclocked])),
                Status::Ok,
                "{nominal:?}"
            );
        }
    }

    #[test]
    fn no_boards_at_all_is_low() {
        assert_eq!(
            derive(&readings(TunerState::Stable, Vec::new())),
            Status::Low
        );
        assert_eq!(
            derive(&readings(TunerState::Tuning, Vec::new())),
            Status::Low
        );
    }

    fn shown_after(tracker: &mut StatusTracker, derived: Option<Status>) -> Option<Status> {
        tracker.observe(derived);
        tracker.shown()
    }

    #[test]
    fn tracker_shows_nothing_until_the_first_success_or_the_budget_runs_out() {
        let mut tracker = StatusTracker::default();
        for _ in 1..FAILURES_BEFORE_LOW {
            assert_eq!(shown_after(&mut tracker, None), None);
        }
        assert_eq!(shown_after(&mut tracker, None), Some(Status::Low));
    }

    #[test]
    fn tracker_keeps_the_last_answer_through_the_budget_then_turns_low() {
        let mut tracker = StatusTracker::default();
        assert_eq!(
            shown_after(&mut tracker, Some(Status::Tuning)),
            Some(Status::Tuning)
        );
        for _ in 1..FAILURES_BEFORE_LOW {
            assert_eq!(shown_after(&mut tracker, None), Some(Status::Tuning));
        }
        assert_eq!(shown_after(&mut tracker, None), Some(Status::Low));
        assert_eq!(shown_after(&mut tracker, None), Some(Status::Low));
    }

    #[test]
    fn a_success_resets_the_budget() {
        let mut tracker = StatusTracker::default();
        tracker.observe(Some(Status::Ok));
        for _ in 1..FAILURES_BEFORE_LOW {
            tracker.observe(None);
        }
        assert_eq!(
            shown_after(&mut tracker, Some(Status::Ok)),
            Some(Status::Ok)
        );
        for _ in 1..FAILURES_BEFORE_LOW {
            assert_eq!(shown_after(&mut tracker, None), Some(Status::Ok));
        }
        assert_eq!(shown_after(&mut tracker, None), Some(Status::Low));
    }

    #[test]
    fn a_success_after_exhaustion_recovers_at_once() {
        let mut tracker = StatusTracker::default();
        for _ in 0..FAILURES_BEFORE_LOW {
            tracker.observe(None);
        }
        assert_eq!(
            shown_after(&mut tracker, Some(Status::Ok)),
            Some(Status::Ok)
        );
    }

    #[test]
    fn exhaustion_is_reported_once_on_the_failure_that_uses_up_the_budget() {
        let mut tracker = StatusTracker::default();
        for _ in 1..FAILURES_BEFORE_LOW {
            assert_eq!(tracker.observe(None), Transition::Unchanged);
        }
        assert_eq!(tracker.observe(None), Transition::Exhausted);
        assert_eq!(tracker.observe(None), Transition::Unchanged);
    }

    #[test]
    fn recovery_is_reported_only_for_a_success_that_ends_an_exhausted_run() {
        let mut tracker = StatusTracker::default();
        assert_eq!(tracker.observe(Some(Status::Ok)), Transition::Unchanged);
        tracker.observe(None);
        assert_eq!(
            tracker.observe(Some(Status::Ok)),
            Transition::Unchanged,
            "one failure inside the budget is not an outage"
        );
        for _ in 0..FAILURES_BEFORE_LOW {
            tracker.observe(None);
        }
        assert_eq!(tracker.observe(Some(Status::Ok)), Transition::Recovered);
        assert_eq!(tracker.observe(Some(Status::Ok)), Transition::Unchanged);
    }
}

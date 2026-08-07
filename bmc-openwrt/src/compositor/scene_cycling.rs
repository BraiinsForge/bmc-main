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

//! Automatic scene cycling state machine.

use bmc::compositor::SceneCyclingTransition;
use std::time::{Duration, Instant};

pub(crate) const PRE_TRANSITION_DURATION: Duration = Duration::from_millis(100);
pub(crate) const AUTOMATIC_TRANSITION_DURATION: Duration = Duration::from_millis(300);
const AUTOMATIC_TRANSITION_TICK: Duration = Duration::from_millis(16);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SceneCyclingRuntimeConfig {
    pub(crate) enabled: bool,
    pub(crate) default_duration: Duration,
    pub(crate) transition: SceneCyclingTransition,
}

impl Default for SceneCyclingRuntimeConfig {
    fn default() -> Self {
        Self::from(bmc::compositor::SceneCycling::default())
    }
}

impl From<bmc::compositor::SceneCycling> for SceneCyclingRuntimeConfig {
    fn from(config: bmc::compositor::SceneCycling) -> Self {
        Self {
            enabled: config.automatic_cycling_enabled,
            default_duration: config.automatic_cycling_default_duration,
            transition: config.transition,
        }
    }
}

/// Per-frame animation value of an in-flight automatic transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TransitionFrame {
    /// X-offset of the outgoing scene, `0` down to `-logical_width`;
    /// the incoming scene follows one screen width to the right.
    Slide { offset: i32 },
    /// Cross-fade progress, `0.0` to `1.0`: the incoming scene's opacity,
    /// with the outgoing scene at the complement.
    Fade { progress: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutomaticCyclingPhase {
    PausedDisabled { started_at: Instant },
    WaitingForTimer { started_at: Instant },
    PreTransition { started_at: Instant },
    Transition { started_at: Instant },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutomaticCyclingAction {
    None,
    BeginPreTransition,
    BeginTransition,
    FinishTransition,
}

#[derive(Debug, Clone)]
pub(crate) struct AutomaticCycling {
    config: SceneCyclingRuntimeConfig,
    pending_config: Option<SceneCyclingRuntimeConfig>,
    phase: AutomaticCyclingPhase,
    /// Night-mode gate, separate from the user's `config.enabled`: cycling stops
    /// for the whole of night mode, so the cycler cannot walk away from the scene
    /// the panel will wake on. Unlike a config disable this takes effect at once,
    /// and night mode can start while the panel is still lit — so the caller has
    /// to undo a slide that is already running.
    suspended: bool,
}

impl AutomaticCycling {
    pub(crate) fn new(started_at: Instant, config: SceneCyclingRuntimeConfig) -> Self {
        Self {
            config,
            pending_config: None,
            phase: AutomaticCyclingPhase::PausedDisabled { started_at },
            suspended: false,
        }
    }

    pub(crate) fn set_suspended(&mut self, suspended: bool) {
        self.suspended = suspended;
    }

    pub(crate) fn phase(&self) -> AutomaticCyclingPhase {
        self.phase
    }

    pub(crate) fn default_duration(&self) -> Duration {
        self.config.default_duration
    }

    #[cfg(test)]
    pub(crate) fn transition(&self) -> SceneCyclingTransition {
        self.config.transition
    }

    /// Animation length of the configured effect. `None` is a zero-length
    /// transition: it commits straight from the warmed-up pre-transition
    /// and never enters the `Transition` phase.
    fn transition_duration(&self) -> Duration {
        match self.config.transition {
            SceneCyclingTransition::Slide | SceneCyclingTransition::Fade => {
                AUTOMATIC_TRANSITION_DURATION
            }
            SceneCyclingTransition::None => Duration::ZERO,
        }
    }

    pub(crate) fn set_config(&mut self, config: SceneCyclingRuntimeConfig) {
        if matches!(
            self.phase,
            AutomaticCyclingPhase::PreTransition { .. } | AutomaticCyclingPhase::Transition { .. }
        ) {
            self.pending_config = Some(config);
            return;
        }
        self.pending_config = None;
        self.config = config;
    }

    pub(crate) fn reevaluate(
        &mut self,
        now: Instant,
        has_cycleable_scenes: bool,
        scene_count: usize,
        touch_active: bool,
    ) {
        if self.suspended
            || !self.config.enabled
            || !has_cycleable_scenes
            || scene_count < 2
            || touch_active
        {
            self.phase = AutomaticCyclingPhase::PausedDisabled { started_at: now };
            return;
        }

        if matches!(self.phase, AutomaticCyclingPhase::PausedDisabled { .. }) {
            self.phase = AutomaticCyclingPhase::WaitingForTimer { started_at: now };
        }
    }

    pub(crate) fn reset_waiting(&mut self, now: Instant, scene_count: usize, touch_active: bool) {
        if let Some(config) = self.pending_config.take() {
            self.config = config;
        }
        if !self.suspended && self.config.enabled && scene_count >= 2 && !touch_active {
            self.phase = AutomaticCyclingPhase::WaitingForTimer { started_at: now };
        } else {
            self.phase = AutomaticCyclingPhase::PausedDisabled { started_at: now };
        }
    }

    pub(crate) fn enter_pre_transition(&mut self, now: Instant) {
        self.phase = AutomaticCyclingPhase::PreTransition { started_at: now };
    }

    pub(crate) fn enter_transition(&mut self, now: Instant) {
        self.phase = AutomaticCyclingPhase::Transition { started_at: now };
    }

    pub(crate) fn transition_frame(
        &self,
        logical_width: u32,
        now: Instant,
    ) -> Option<TransitionFrame> {
        let AutomaticCyclingPhase::Transition { started_at } = self.phase else {
            return None;
        };
        let elapsed = now.saturating_duration_since(started_at);
        let progress =
            (elapsed.as_secs_f32() / AUTOMATIC_TRANSITION_DURATION.as_secs_f32()).min(1.0);
        match self.config.transition {
            SceneCyclingTransition::Slide => {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "transition pixel offset is panel-sized"
                )]
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "transition pixel offset is panel-sized"
                )]
                let offset = -((logical_width as f32) * progress).round() as i32;
                Some(TransitionFrame::Slide { offset })
            }
            SceneCyclingTransition::Fade => Some(TransitionFrame::Fade { progress }),
            // None commits straight from PreTransition and never enters this phase.
            SceneCyclingTransition::None => None,
        }
    }

    pub(crate) fn next_delay(&self, now: Instant, active_duration: Duration) -> Option<Duration> {
        match self.phase {
            AutomaticCyclingPhase::PausedDisabled { .. } => None,
            AutomaticCyclingPhase::WaitingForTimer { started_at } => {
                Some(remaining_delay(now, started_at, active_duration))
            }
            AutomaticCyclingPhase::PreTransition { started_at, .. } => {
                Some(remaining_delay(now, started_at, PRE_TRANSITION_DURATION))
            }
            AutomaticCyclingPhase::Transition { .. } => Some(AUTOMATIC_TRANSITION_TICK),
        }
    }

    pub(crate) fn on_timer(
        &mut self,
        now: Instant,
        active_duration: Duration,
    ) -> AutomaticCyclingAction {
        match self.phase {
            AutomaticCyclingPhase::PausedDisabled { .. } => AutomaticCyclingAction::None,
            AutomaticCyclingPhase::WaitingForTimer { started_at } => {
                if now.saturating_duration_since(started_at) >= active_duration {
                    AutomaticCyclingAction::BeginPreTransition
                } else {
                    AutomaticCyclingAction::None
                }
            }
            AutomaticCyclingPhase::PreTransition { started_at, .. } => {
                if now.saturating_duration_since(started_at) >= PRE_TRANSITION_DURATION {
                    if self.transition_duration().is_zero() {
                        AutomaticCyclingAction::FinishTransition
                    } else {
                        AutomaticCyclingAction::BeginTransition
                    }
                } else {
                    AutomaticCyclingAction::None
                }
            }
            AutomaticCyclingPhase::Transition { started_at, .. } => {
                if now.saturating_duration_since(started_at) >= self.transition_duration() {
                    AutomaticCyclingAction::FinishTransition
                } else {
                    AutomaticCyclingAction::None
                }
            }
        }
    }
}

fn remaining_delay(now: Instant, started_at: Instant, duration: Duration) -> Duration {
    duration.saturating_sub(now.saturating_duration_since(started_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn cycling_config(enabled: bool) -> SceneCyclingRuntimeConfig {
        SceneCyclingRuntimeConfig {
            enabled,
            default_duration: Duration::from_secs(30),
            transition: SceneCyclingTransition::Slide,
        }
    }

    fn cycling_config_with_transition(
        transition: SceneCyclingTransition,
    ) -> SceneCyclingRuntimeConfig {
        SceneCyclingRuntimeConfig {
            transition,
            ..cycling_config(true)
        }
    }

    #[test]
    fn automatic_cycling_disabled_enters_paused_disabled() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(false));

        state.reevaluate(now, true, 2, false);

        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
    }

    #[test]
    fn automatic_cycling_enabled_waits_for_timer_with_two_scenes() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));

        state.reevaluate(now, true, 2, false);

        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::WaitingForTimer { .. }
        ));
        assert_eq!(
            state.next_delay(now, Duration::from_secs(30)),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            state.next_delay(now + Duration::from_secs(10), Duration::from_secs(30)),
            Some(Duration::from_secs(20)),
        );
    }

    #[test]
    fn disabling_during_waiting_pauses_immediately() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.reevaluate(now, true, 2, false);

        state.set_config(cycling_config(false));
        state.reevaluate(now, true, 2, false);

        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
        assert_eq!(state.next_delay(now, Duration::from_secs(30)), None);
    }

    #[test]
    fn automatic_cycling_advances_from_waiting_to_pre_transition() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));

        state.reevaluate(now, true, 2, false);

        let action = state.on_timer(now + Duration::from_secs(30), Duration::from_secs(30));

        assert_eq!(action, AutomaticCyclingAction::BeginPreTransition);
    }

    #[test]
    fn automatic_cycling_pre_transition_waits_before_slide() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.enter_pre_transition(now);

        assert_eq!(
            state.on_timer(now + PRE_TRANSITION_DURATION, Duration::from_secs(30)),
            AutomaticCyclingAction::BeginTransition
        );
    }

    #[test]
    fn automatic_cycling_transition_finishes_after_duration() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.enter_transition(now);

        assert_eq!(
            state.on_timer(now + AUTOMATIC_TRANSITION_DURATION, Duration::from_secs(30)),
            AutomaticCyclingAction::FinishTransition
        );
    }

    #[test]
    fn automatic_cycling_slide_frame_follows_elapsed_progress() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.enter_transition(now);

        assert_eq!(
            state.transition_frame(1000, now),
            Some(TransitionFrame::Slide { offset: 0 })
        );
        assert_eq!(
            state.transition_frame(1000, now + Duration::from_millis(150)),
            Some(TransitionFrame::Slide { offset: -500 }),
        );
        assert_eq!(
            state.transition_frame(1000, now + Duration::from_millis(600)),
            Some(TransitionFrame::Slide { offset: -1000 }),
        );
    }

    #[test]
    fn automatic_cycling_fade_frame_follows_elapsed_progress() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(
            now,
            cycling_config_with_transition(SceneCyclingTransition::Fade),
        );
        state.enter_transition(now);

        assert_eq!(
            state.transition_frame(1000, now),
            Some(TransitionFrame::Fade { progress: 0.0 })
        );
        assert_eq!(
            state.transition_frame(1000, now + Duration::from_millis(150)),
            Some(TransitionFrame::Fade { progress: 0.5 }),
        );
        assert_eq!(
            state.transition_frame(1000, now + Duration::from_millis(600)),
            Some(TransitionFrame::Fade { progress: 1.0 }),
        );
    }

    #[test]
    fn automatic_cycling_none_transition_yields_no_frame() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(
            now,
            cycling_config_with_transition(SceneCyclingTransition::None),
        );
        state.enter_transition(now);

        assert_eq!(
            state.transition_frame(1000, now + Duration::from_millis(150)),
            None
        );
    }

    #[test]
    fn no_transition_frame_outside_transition_phase() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(
            now,
            cycling_config_with_transition(SceneCyclingTransition::Fade),
        );
        state.enter_pre_transition(now);

        assert_eq!(state.transition_frame(1000, now), None);
    }

    #[test]
    fn disabling_during_pre_transition_waits_until_next_wait_period() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.enter_pre_transition(now);

        state.set_config(cycling_config(false));
        state.reevaluate(now, true, 2, false);

        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::PreTransition { .. }
        ));
        assert_eq!(
            state.on_timer(now + PRE_TRANSITION_DURATION, Duration::from_secs(30)),
            AutomaticCyclingAction::BeginTransition,
        );

        state.enter_transition(now + PRE_TRANSITION_DURATION);
        assert_eq!(
            state.on_timer(
                now + PRE_TRANSITION_DURATION + AUTOMATIC_TRANSITION_DURATION,
                Duration::from_secs(30)
            ),
            AutomaticCyclingAction::FinishTransition,
        );

        state.reset_waiting(
            now + PRE_TRANSITION_DURATION + AUTOMATIC_TRANSITION_DURATION,
            2,
            false,
        );
        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
    }

    #[test]
    fn suspension_pauses_and_resuming_restarts_waiting() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.reevaluate(now, true, 2, false);
        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::WaitingForTimer { .. }
        ));

        // Screen off: suspension pauses immediately and stops the timer.
        state.set_suspended(true);
        state.reevaluate(now, true, 2, false);
        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
        assert_eq!(state.next_delay(now, Duration::from_secs(30)), None);

        // Wake: resume restarts a fresh waiting period.
        state.set_suspended(false);
        let later = now + Duration::from_secs(3600);
        state.reevaluate(later, true, 2, false);
        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::WaitingForTimer { .. }
        ));
        assert_eq!(
            state.next_delay(later, Duration::from_secs(30)),
            Some(Duration::from_secs(30)),
            "waiting must restart from resume, not from before the suspend"
        );
    }

    #[test]
    fn reset_waiting_stays_paused_while_suspended() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.set_suspended(true);

        state.reset_waiting(now, 2, false);

        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
        assert_eq!(state.next_delay(now, Duration::from_secs(30)), None);
    }

    #[test]
    fn disabling_during_transition_waits_until_next_wait_period() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.enter_transition(now);

        state.set_config(cycling_config(false));
        state.reevaluate(now, true, 2, false);

        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::Transition { .. }
        ));
        assert_eq!(
            state.on_timer(now + AUTOMATIC_TRANSITION_DURATION, Duration::from_secs(30)),
            AutomaticCyclingAction::FinishTransition,
        );

        state.reset_waiting(now + AUTOMATIC_TRANSITION_DURATION, 2, false);
        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
    }

    #[test]
    fn none_transition_finishes_straight_from_pre_transition() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(
            now,
            cycling_config_with_transition(SceneCyclingTransition::None),
        );
        state.enter_pre_transition(now);

        assert_eq!(
            state.on_timer(now + PRE_TRANSITION_DURATION, Duration::from_secs(30)),
            AutomaticCyclingAction::FinishTransition,
        );
    }

    #[test]
    fn transition_change_during_transition_defers_until_next_wait_period() {
        let now = Instant::now();
        let mut state = AutomaticCycling::new(now, cycling_config(true));
        state.enter_transition(now);

        state.set_config(cycling_config_with_transition(SceneCyclingTransition::Fade));

        assert_eq!(state.transition(), SceneCyclingTransition::Slide);
        assert_eq!(
            state.transition_frame(1000, now + Duration::from_millis(150)),
            Some(TransitionFrame::Slide { offset: -500 })
        );

        state.reset_waiting(now + AUTOMATIC_TRANSITION_DURATION, 2, false);
        assert_eq!(state.transition(), SceneCyclingTransition::Fade);
        assert!(matches!(
            state.phase(),
            AutomaticCyclingPhase::WaitingForTimer { .. }
        ));
    }
}

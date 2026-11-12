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

//! The card action under the tap and hold settings: a tap fires it, a hold fires it once filled,
//! or both, where a hold let go early is still a tap.
//!
//! Time enters only as each frame's `delta_ms`,
//! so a hold is measured over the frames the widget draws while it lasts.

use std::num::NonZeroU32;

/// What a tap on the card does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnTap {
    Fires,
    Nothing,
}

/// What a finger did on the action, as one frame's touches report it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Touch {
    /// A finger is on it now.
    pub held: bool,
    /// A finger lifted from it since the last frame.
    pub released: bool,
}

/// Whether the action can fire now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Ready,
    /// Such as its call still on its way, or no connection: a touch on it fires nothing.
    Blocked,
}

/// What a frame's touch came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Nothing,
    Fired,
    /// A hold let go before it filled, where a tap does not fire.
    ReleasedEarly,
}

/// Where the finger is, as the last frame left it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finger {
    Up,
    /// Down since the action could fire, with no hold set: lifting it is a tap.
    Tapping,
    /// Down, and nothing to come of it: a hold that fired, or a finger that landed while blocked.
    Down,
    /// Down and filling a hold of `hold_ms`, for `held_ms` so far.
    Filling {
        held_ms: u32,
        hold_ms: NonZeroU32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gesture {
    tap: OnTap,
    /// How long a hold takes to fire the action; none where holding fires nothing.
    hold_ms: Option<NonZeroU32>,
    finger: Finger,
}

impl Gesture {
    #[must_use]
    pub const fn new(tap: OnTap, hold_ms: Option<NonZeroU32>) -> Self {
        Self {
            tap,
            hold_ms,
            finger: Finger::Up,
        }
    }

    /// Take new tap and hold settings, keeping the finger: a hold that was filling stops, firing nothing.
    pub fn set(&mut self, tap: OnTap, hold_ms: Option<NonZeroU32>) {
        self.tap = tap;
        self.hold_ms = hold_ms;
        if matches!(self.finger, Finger::Filling { .. }) {
            self.finger = Finger::Down;
        }
    }

    /// Take the settings of another action, which a finger already down never fires.
    pub fn set_for_another_action(&mut self, tap: OnTap, hold_ms: Option<NonZeroU32>) {
        self.set(tap, hold_ms);
        if self.finger != Finger::Up {
            self.finger = Finger::Down;
        }
    }

    /// Take the touch of a frame drawn `delta_ms` after the last one.
    /// A blocked action ends a hold but keeps following the finger,
    /// so only a finger that lands while it can fire starts a tap or a hold, never one still down from before.
    pub fn frame(&mut self, delta_ms: u32, touch: Touch, availability: Availability) -> Step {
        let was = self.finger;
        if availability == Availability::Blocked {
            self.finger = if touch.held { Finger::Down } else { Finger::Up };
            return Step::Nothing;
        }
        match (was, touch.held) {
            (Finger::Filling { held_ms, hold_ms }, true) => {
                let held_ms = held_ms.saturating_add(delta_ms);
                if held_ms >= hold_ms.get() {
                    self.finger = Finger::Down;
                    Step::Fired
                } else {
                    self.finger = Finger::Filling { held_ms, hold_ms };
                    Step::Nothing
                }
            }
            (Finger::Up, true) => {
                self.finger = self.landed();
                Step::Nothing
            }
            (Finger::Tapping | Finger::Down, true) => Step::Nothing,
            // A tap shorter than a frame, never seen held.
            (Finger::Up, false) if touch.released => self.lifted(self.landed()),
            (finger, false) => {
                self.finger = Finger::Up;
                if touch.released {
                    self.lifted(finger)
                } else {
                    Step::Nothing
                }
            }
        }
    }

    /// Where a finger that lands now starts.
    /// The first frame of a hold adds no time: its delta covers the idle before it.
    fn landed(&self) -> Finger {
        match self.hold_ms {
            None => Finger::Tapping,
            Some(hold_ms) => Finger::Filling {
                held_ms: 0,
                hold_ms,
            },
        }
    }

    /// What a finger lifted from where `finger` was comes to.
    fn lifted(&self, finger: Finger) -> Step {
        match (finger, self.tap) {
            (Finger::Tapping | Finger::Filling { .. }, OnTap::Fires) => Step::Fired,
            (Finger::Filling { .. }, OnTap::Nothing) => Step::ReleasedEarly,
            (Finger::Tapping, OnTap::Nothing) | (Finger::Up | Finger::Down, _) => Step::Nothing,
        }
    }

    /// How far a hold has filled, from 0 to 1, while it does.
    #[must_use]
    pub fn progress(&self) -> Option<f32> {
        let Finger::Filling { held_ms, hold_ms } = self.finger else {
            return None;
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "a hold is a few thousand ms, exact in f32"
        )]
        Some(held_ms as f32 / hold_ms.get() as f32)
    }

    /// A hold is filling: it needs a frame each frame, as a still finger reports nothing.
    #[must_use]
    pub fn filling(&self) -> bool {
        matches!(self.finger, Finger::Filling { .. })
    }

    /// How long a hold takes, where only a hold fires the action.
    #[must_use]
    pub fn hold_only_ms(&self) -> Option<NonZeroU32> {
        self.hold_ms.filter(|_| self.tap == OnTap::Nothing)
    }

    /// Whether a tap or a hold fires the action at all.
    #[must_use]
    pub fn can_fire(&self) -> bool {
        self.tap == OnTap::Fires || self.hold_ms.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    const HELD: Touch = Touch {
        held: true,
        released: false,
    };
    const LIFTED: Touch = Touch {
        held: false,
        released: true,
    };
    const IDLE: Touch = Touch {
        held: false,
        released: false,
    };
    const READY: Availability = Availability::Ready;
    const BLOCKED: Availability = Availability::Blocked;
    const HOLD: Option<NonZeroU32> = NonZeroU32::new(1_000);

    #[wasm_test]
    fn a_tap_fires_without_a_hold() {
        let mut gesture = Gesture::new(OnTap::Fires, None);
        assert_eq!(gesture.frame(16, HELD, READY), Step::Nothing);
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::Fired);
    }

    #[wasm_test]
    fn a_hold_fires_once_filled_while_the_finger_is_still_down() {
        let mut gesture = Gesture::new(OnTap::Nothing, HOLD);
        assert_eq!(
            gesture.frame(5_000, HELD, READY),
            Step::Nothing,
            "the idle before it"
        );
        assert_eq!(gesture.frame(600, HELD, READY), Step::Nothing);
        assert_eq!(gesture.progress(), Some(0.6));
        assert_eq!(gesture.frame(400, HELD, READY), Step::Fired);
        assert_eq!(
            gesture.frame(400, HELD, READY),
            Step::Nothing,
            "one firing per hold"
        );
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::Nothing);
    }

    #[wasm_test]
    fn a_finger_still_down_after_its_call_was_answered_fires_nothing_more() {
        let mut gesture = Gesture::new(OnTap::Fires, HOLD);
        gesture.frame(16, HELD, READY);
        assert_eq!(gesture.frame(1_000, HELD, READY), Step::Fired);
        assert_eq!(
            gesture.frame(16, HELD, BLOCKED),
            Step::Nothing,
            "on its way"
        );
        assert_eq!(gesture.frame(1_000, HELD, READY), Step::Nothing, "answered");
        assert_eq!(gesture.progress(), None, "no second hold starts");
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::Nothing);
    }

    #[wasm_test]
    fn a_hold_let_go_early_is_a_tap_where_taps_fire() {
        let mut gesture = Gesture::new(OnTap::Fires, HOLD);
        gesture.frame(16, HELD, READY);
        gesture.frame(300, HELD, READY);
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::Fired);
    }

    #[wasm_test]
    fn a_hold_let_go_early_only_says_so_where_taps_do_not_fire() {
        let mut gesture = Gesture::new(OnTap::Nothing, HOLD);
        gesture.frame(16, HELD, READY);
        gesture.frame(300, HELD, READY);
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::ReleasedEarly);
        assert_eq!(gesture.progress(), None);
    }

    #[wasm_test]
    fn a_tap_shorter_than_a_frame_still_counts() {
        for hold_ms in [None, HOLD] {
            let mut gesture = Gesture::new(OnTap::Fires, hold_ms);
            assert_eq!(
                gesture.frame(16, LIFTED, READY),
                Step::Fired,
                "hold {hold_ms:?}"
            );
        }
    }

    #[wasm_test]
    fn a_hold_let_go_within_a_frame_only_says_so_where_taps_do_not_fire() {
        let mut gesture = Gesture::new(OnTap::Nothing, HOLD);
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::ReleasedEarly);
    }

    #[wasm_test]
    fn a_finger_slid_off_does_not_fire() {
        let mut gesture = Gesture::new(OnTap::Fires, HOLD);
        gesture.frame(16, HELD, READY);
        assert_eq!(gesture.frame(16, IDLE, READY), Step::Nothing);
    }

    #[wasm_test]
    fn a_blocked_action_ends_a_hold_and_only_a_new_finger_starts_one() {
        let mut gesture = Gesture::new(OnTap::Nothing, HOLD);
        gesture.frame(16, HELD, READY);
        gesture.frame(500, HELD, READY);
        assert_eq!(gesture.frame(16, HELD, BLOCKED), Step::Nothing);
        assert_eq!(gesture.progress(), None);
        assert_eq!(gesture.frame(600, HELD, READY), Step::Nothing);
        assert_eq!(gesture.progress(), None, "the same finger starts nothing");
        gesture.frame(16, IDLE, READY);
        gesture.frame(16, HELD, READY);
        assert_eq!(gesture.progress(), Some(0.0), "a new finger does");
    }

    #[wasm_test]
    fn a_finger_that_landed_while_blocked_fires_nothing_when_lifted() {
        for hold_ms in [None, HOLD] {
            let mut gesture = Gesture::new(OnTap::Fires, hold_ms);
            gesture.frame(16, HELD, BLOCKED);
            gesture.frame(16, HELD, READY);
            assert_eq!(
                gesture.frame(16, LIFTED, READY),
                Step::Nothing,
                "a second tap while the first is on its way, hold {hold_ms:?}"
            );
        }
    }

    #[wasm_test]
    fn new_settings_start_no_second_hold_under_a_finger_still_down() {
        let mut gesture = Gesture::new(OnTap::Nothing, HOLD);
        gesture.frame(16, HELD, READY);
        assert_eq!(gesture.frame(1_000, HELD, READY), Step::Fired);
        gesture.set(OnTap::Nothing, HOLD);
        assert_eq!(gesture.frame(1_000, HELD, READY), Step::Nothing);
        assert_eq!(gesture.progress(), None);
    }

    #[wasm_test]
    fn new_settings_keep_a_finger_that_landed_while_blocked_from_firing() {
        let mut gesture = Gesture::new(OnTap::Fires, None);
        gesture.frame(16, HELD, BLOCKED);
        gesture.set(OnTap::Fires, None);
        gesture.frame(16, HELD, READY);
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::Nothing);
    }

    #[wasm_test]
    fn new_settings_stop_a_filling_hold_without_firing() {
        let mut gesture = Gesture::new(OnTap::Fires, HOLD);
        gesture.frame(16, HELD, READY);
        gesture.frame(500, HELD, READY);
        gesture.set(OnTap::Fires, NonZeroU32::new(400));
        assert_eq!(gesture.progress(), None);
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::Nothing);
    }

    #[wasm_test]
    fn another_action_fires_nothing_for_a_finger_already_down() {
        for hold_ms in [None, HOLD] {
            let mut gesture = Gesture::new(OnTap::Fires, hold_ms);
            gesture.frame(16, HELD, READY);
            gesture.frame(500, HELD, READY);
            gesture.set_for_another_action(OnTap::Fires, hold_ms);
            assert_eq!(gesture.frame(1_000, HELD, READY), Step::Nothing);
            assert_eq!(gesture.progress(), None, "hold {hold_ms:?}");
            assert_eq!(
                gesture.frame(16, LIFTED, READY),
                Step::Nothing,
                "hold {hold_ms:?}"
            );
        }
    }

    #[wasm_test]
    fn another_action_fires_for_a_finger_that_lands_on_it() {
        let mut gesture = Gesture::new(OnTap::Fires, None);
        gesture.set_for_another_action(OnTap::Fires, None);
        gesture.frame(16, HELD, READY);
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::Fired);
    }

    #[wasm_test]
    fn with_both_off_nothing_fires() {
        let mut gesture = Gesture::new(OnTap::Nothing, None);
        assert!(!gesture.can_fire());
        gesture.frame(16, HELD, READY);
        assert_eq!(gesture.frame(16, LIFTED, READY), Step::Nothing);
    }
}

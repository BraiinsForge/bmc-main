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

//! A line on the tile telling the user what became of something they did, gone after a while.
//!
//! Time enters only as each frame's `delta_ms`. A notice arrives between frames,
//! so the first frame that shows it counts none of the time before it.

/// How long a call that failed is told.
pub const FAILURE_MS: u32 = 4_000;
/// How long a hold let go before it filled is told.
pub const RELEASED_EARLY_MS: u32 = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Clock {
    /// No frame has shown it yet: it lasts `shown_ms` from the first that does.
    Waiting {
        shown_ms: u32,
    },
    Running {
        left_ms: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    text: String,
    clock: Clock,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notice {
    shown: Option<Shown>,
}

impl Notice {
    /// Tell `text` for `shown_ms` from the next frame, in place of what was told.
    pub fn show(&mut self, text: String, shown_ms: u32) {
        self.shown = Some(Shown {
            text,
            clock: Clock::Waiting { shown_ms },
        });
    }

    pub fn clear(&mut self) {
        self.shown = None;
    }

    /// Let `delta_ms` pass before a frame is drawn, so a notice that ran out is not drawn again.
    pub fn tick(&mut self, delta_ms: u32) {
        let Some(Shown { clock, .. }) = &mut self.shown else {
            return;
        };
        let left_ms = match *clock {
            Clock::Waiting { shown_ms } => shown_ms,
            Clock::Running { left_ms } => left_ms.saturating_sub(delta_ms),
        };
        if left_ms == 0 {
            self.shown = None;
        } else {
            *clock = Clock::Running { left_ms };
        }
    }

    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.shown.as_ref().map(|shown| shown.text.as_str())
    }

    /// How long until it goes.
    #[must_use]
    pub fn left_ms(&self) -> Option<u32> {
        match self.shown.as_ref()?.clock {
            Clock::Waiting { shown_ms } => Some(shown_ms),
            Clock::Running { left_ms } => Some(left_ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn a_notice_lasts_its_time_from_the_first_frame_that_shows_it() {
        let mut notice = Notice::default();
        notice.show("Failed".to_owned(), 4_000);
        notice.tick(60_000);
        assert_eq!(
            notice.text(),
            Some("Failed"),
            "the idle before it does not count"
        );
        notice.tick(3_000);
        assert_eq!(notice.left_ms(), Some(1_000));
        notice.tick(1_000);
        assert_eq!(notice.text(), None);
    }

    #[wasm_test]
    fn a_cleared_notice_is_gone_at_once() {
        let mut notice = Notice::default();
        notice.show("Failed".to_owned(), 4_000);
        notice.tick(16);
        notice.clear();
        assert_eq!(notice.left_ms(), None);
    }

    #[wasm_test]
    fn a_new_notice_replaces_the_one_told() {
        let mut notice = Notice::default();
        notice.show("First".to_owned(), 4_000);
        notice.tick(16);
        notice.tick(3_000);
        notice.show("Second".to_owned(), 2_000);
        notice.tick(16);
        assert_eq!(notice.text(), Some("Second"));
        assert_eq!(notice.left_ms(), Some(2_000));
    }
}

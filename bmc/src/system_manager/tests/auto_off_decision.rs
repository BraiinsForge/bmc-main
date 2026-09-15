// Copyright (C) 2025  Braiins Systems s.r.o.
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

use crate::system_manager::{AutoOffMode, MIN_SCREEN_OFF_TIMEOUT_SECS, auto_off_decision};

#[test]
fn ringing_alarm_keeps_screen_on_even_with_night_mode_timeout() {
    // The acceptance criterion: an active alarm must never let the screen
    // auto-off, regardless of night mode or a configured timeout.
    assert_eq!(
        auto_off_decision(true, true, Some(60)),
        AutoOffMode::KeepOn,
        "ringing alarm must inhibit auto-off"
    );
}

#[test]
fn night_mode_with_timeout_arms_timer_when_not_ringing() {
    assert_eq!(
        auto_off_decision(true, false, Some(60)),
        AutoOffMode::ArmTimer(Duration::from_mins(1))
    );
}

#[test]
fn timeout_below_minimum_is_clamped() {
    assert_eq!(
        auto_off_decision(true, false, Some(1)),
        AutoOffMode::ArmTimer(Duration::from_secs(u64::from(MIN_SCREEN_OFF_TIMEOUT_SECS)))
    );
}

#[test]
fn no_night_mode_or_no_timeout_keeps_screen_on() {
    assert_eq!(
        auto_off_decision(false, false, Some(60)),
        AutoOffMode::KeepOn
    );
    assert_eq!(auto_off_decision(true, false, Some(0)), AutoOffMode::KeepOn);
    assert_eq!(auto_off_decision(true, false, None), AutoOffMode::KeepOn);
}

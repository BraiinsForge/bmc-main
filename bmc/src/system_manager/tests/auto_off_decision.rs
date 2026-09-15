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

use crate::system_manager::{
    AutoOffInputs, AutoOffMode, MIN_SCREEN_OFF_TIMEOUT_SECS, ScreenRequest, auto_off_decision,
};

/// Night mode on with a minute's timeout, the panel lit and no alarm:
/// the state the timer arms in, and the one every test below departs from.
fn armed_night() -> AutoOffInputs {
    AutoOffInputs {
        night_mode_active: true,
        alarm_ringing: false,
        timeout_secs: Some(60),
        request: ScreenRequest::Wake,
        timer_blanked: false,
    }
}

#[test]
fn ringing_alarm_keeps_screen_on_even_with_night_mode_timeout() {
    // The acceptance criterion: an active alarm must never let the screen
    // auto-off, regardless of night mode or a configured timeout.
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            alarm_ringing: true,
            ..armed_night()
        }),
        AutoOffMode::KeepOn,
        "ringing alarm must inhibit auto-off"
    );
}

#[test]
fn night_mode_with_timeout_arms_timer_when_not_ringing() {
    assert_eq!(
        auto_off_decision(armed_night()),
        AutoOffMode::ArmTimer(Duration::from_mins(1))
    );
}

#[test]
fn timeout_below_minimum_is_clamped() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            timeout_secs: Some(1),
            ..armed_night()
        }),
        AutoOffMode::ArmTimer(Duration::from_secs(u64::from(MIN_SCREEN_OFF_TIMEOUT_SECS)))
    );
}

#[test]
fn no_night_mode_or_no_timeout_keeps_screen_on() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            night_mode_active: false,
            ..armed_night()
        }),
        AutoOffMode::KeepOn
    );
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            timeout_secs: Some(0),
            ..armed_night()
        }),
        AutoOffMode::KeepOn,
        "a zero persisted before the setter mapped it to None still means Never"
    );
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            timeout_secs: None,
            ..armed_night()
        }),
        AutoOffMode::KeepOn
    );
}

#[test]
fn a_user_blank_holds_dark_outside_night_mode() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            night_mode_active: false,
            timeout_secs: None,
            request: ScreenRequest::Blank,
            ..armed_night()
        }),
        AutoOffMode::HoldDark,
        "the KeepOn wake would undo the blank the user asked for"
    );
}

#[test]
fn a_user_blank_holds_dark_over_the_auto_off_timer() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            request: ScreenRequest::Blank,
            ..armed_night()
        }),
        AutoOffMode::HoldDark
    );
}

#[test]
fn a_ringing_alarm_overrides_a_user_blank() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            night_mode_active: false,
            alarm_ringing: true,
            timeout_secs: None,
            request: ScreenRequest::Blank,
            ..armed_night()
        }),
        AutoOffMode::KeepOn,
        "a firing alarm must never sit on a panel the user blanked"
    );
}

#[test]
fn the_timers_own_blank_holds_dark_while_night_mode_lasts() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            timer_blanked: true,
            ..armed_night()
        }),
        AutoOffMode::HoldDark,
        "re-arming the timer would blank an already dark panel every timeout"
    );
}

#[test]
fn the_timers_own_blank_does_not_outlive_night_mode() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            night_mode_active: false,
            timer_blanked: true,
            ..armed_night()
        }),
        AutoOffMode::KeepOn
    );
}

#[test]
fn a_timeout_of_never_ends_the_timers_own_blank() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            timeout_secs: None,
            timer_blanked: true,
            ..armed_night()
        }),
        AutoOffMode::KeepOn,
        "the story promises that Never keeps the screen on during night mode"
    );
}

#[test]
fn a_ringing_alarm_overrides_the_timers_own_blank() {
    assert_eq!(
        auto_off_decision(AutoOffInputs {
            alarm_ringing: true,
            timer_blanked: true,
            ..armed_night()
        }),
        AutoOffMode::KeepOn
    );
}

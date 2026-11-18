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

//! Sizes, settings and contexts to stage a tile at, off-device.

use std::num::NonZeroU32;

use bmc_wasm_sdk::WidgetSize;
use home_assistant::{CallFailure, HaError};

use crate::interaction::gesture::{Availability, Gesture, OnTap, Touch};
use crate::interaction::pending::Pending;
use crate::present::{self, Context, Reach, Room};

/// The sizes a tile takes on a BMC100: the combined scene's slots, and a fullscreen scene.
pub const SIZES: [((u32, u32), &str); 4] = [
    ((317, 238), "1x1"),
    ((638, 238), "2x1"),
    ((638, 480), "2x2"),
    ((1280, 480), "Full"),
];
/// 2x1, which the other knobs are captured at.
pub const DEFAULT_SIZE: usize = 1;
pub const DEFAULT_DIMENSIONS: (u32, u32) = SIZES[DEFAULT_SIZE].0;

const HOLD_MS: u32 = 2_000;
pub const HOLD: Option<NonZeroU32> = NonZeroU32::new(HOLD_MS);
pub const SETTINGS: [((OnTap, Option<NonZeroU32>), &str); 4] = [
    ((OnTap::Fires, None), "Tap"),
    ((OnTap::Nothing, HOLD), "Hold 2 s"),
    ((OnTap::Fires, HOLD), "Tap or hold"),
    ((OnTap::Nothing, None), "Neither"),
];

/// What the line tells in place of the entity's state, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Told {
    State,
    Notice,
    /// A call Home Assistant refused, for the longest notice a failed call shows.
    Refused,
    Reconnecting,
}

pub const LINES: [(Told, &str); 4] = [
    (Told::State, "Status"),
    (Told::Notice, "Notice"),
    (Told::Refused, "Refused"),
    (Told::Reconnecting, "Reconnecting"),
];

impl Told {
    /// The notice and reach it tells over `reach`.
    #[must_use]
    pub fn over(self, reach: Reach) -> (Option<&'static str>, Reach) {
        match self {
            Self::State => (None, reach),
            Self::Notice => (Some("Released too early"), reach),
            Self::Refused => {
                let refused = CallFailure::Rejected(HaError {
                    code: String::new(),
                    message: String::new(),
                });
                (Some(present::failure_notice(&refused)), reach)
            }
            Self::Reconnecting => (None, Reach::Reconnecting),
        }
    }
}

/// `gesture` held for half of [`HOLD`].
pub fn hold_half_way(gesture: &mut Gesture) {
    const HELD: Touch = Touch {
        held: true,
        released: false,
    };
    gesture.frame(0, HELD, Availability::Ready);
    gesture.frame(HOLD_MS / 2, HELD, Availability::Ready);
}

/// The room of a tile of `size`, one of [`SIZES`].
#[must_use]
pub fn room((width, height): (u32, u32)) -> Room {
    #[expect(clippy::cast_precision_loss, reason = "a tile at most 1280 px across")]
    Room {
        width: width as f32,
        height: height as f32,
        size: WidgetSize::from_dimensions(width, height).variant,
    }
}

/// A tile of `size`, one of [`SIZES`], named `name`, with the line telling what [`Told::over`] gives.
#[must_use]
pub fn context<'a>(
    pending: &'a Pending,
    gesture: &'a Gesture,
    name: &'a str,
    (notice, reach): (Option<&'a str>, Reach),
    size: (u32, u32),
) -> Context<'a> {
    Context {
        pending,
        gesture,
        name,
        notice,
        reach,
        room: room(size),
    }
}

/// The tests' tile: a gate at the size captures use, its line telling the state over `reach`.
#[cfg(test)]
#[must_use]
pub fn gate<'a>(pending: &'a Pending, gesture: &'a Gesture, reach: Reach) -> Context<'a> {
    context(pending, gesture, "Gate", (None, reach), DEFAULT_DIMENSIONS)
}

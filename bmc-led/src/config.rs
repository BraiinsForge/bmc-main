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

use crate::data::Rgb;

pub const LED_COUNT: u8 = 10;
pub const LED_MAX_BRIGHTNESS: f32 = 1.0;

pub const RGB_VIOLET60: Rgb = Rgb::new(107, 80, 255);
pub const RGB_GREEN: Rgb = Rgb::new(0, 255, 0);
pub const RGB_RED: Rgb = Rgb::new(255, 0, 0);
pub const RGB_ORANGE: Rgb = Rgb::new(255, 122, 13);
pub const RGB_BLACK: Rgb = Rgb::new(0, 0, 0);
pub const RGB_WHITE: Rgb = Rgb::new(255, 255, 255);

pub const ERROR_DURATION: Duration = Duration::from_secs(2);
pub const SUCCESS_DURATION: Duration = Duration::from_secs(2);

pub const SNAKE_PERIOD: Duration = Duration::from_secs(1);
pub const KNIGHT_RIDER_PERIOD: Duration = Duration::from_secs(1);
pub const BREATHE_PERIOD: Duration = Duration::from_secs(4);

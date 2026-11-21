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

//! Buttons an integration provides, whose state is the time of their last press.

use super::known_state;
use crate::{Entity, ServiceCall};

pub(super) const DOMAIN: &str = "button";
pub const ATTRIBUTES: &[&str] = &[];

#[must_use]
pub fn press() -> ServiceCall {
    ServiceCall::new(DOMAIN, "press")
}

/// When the button was last pressed, as Home Assistant's ISO 8601 timestamp.
#[must_use]
pub fn last_pressed(button: &Entity) -> Option<&str> {
    known_state(button)
}

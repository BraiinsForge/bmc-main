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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootloaderConfig {
    /// Night mode start time in UTC minutes. None means night mode is disabled.
    pub night_from_utc_minutes: Option<u16>,
    /// Night mode end time in UTC minutes. None means night mode is disabled.
    pub night_to_utc_minutes: Option<u16>,
    /// LED enabled during day
    pub led_day: bool,
    /// LED enabled during night. None means night mode is disabled.
    pub led_night: Option<bool>,
    /// Screen brightness during day (actual hardware value, not percentage)
    pub screen_day: u8,
    /// Screen brightness during night (actual hardware value, not percentage). None means night mode is disabled.
    pub screen_night: Option<u8>,
}

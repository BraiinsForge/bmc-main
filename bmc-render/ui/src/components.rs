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

//! Builders that compose the tree's nodes into the design system's components.
//!
//! Pure: a builder here turns its arguments into a [`crate::Node`] and
//! nothing else. Registering an icon or reading the clock stays with the SDK,
//! which passes the result in as an id or a [`crate::SystemTime`].

pub mod modal;
pub mod notification;
pub mod number_input;
pub mod progress_bar;
pub mod relative_time;
pub mod skeleton;
pub mod status_overlay;
pub mod switcher;
pub mod tag;
pub mod text;

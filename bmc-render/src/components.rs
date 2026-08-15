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

//! UI components built on top of drawing and interaction primitives.

mod button;
pub(crate) mod draw;
pub(crate) mod modal;
pub(crate) mod notification;
pub(crate) mod progress_bar;
pub(crate) mod relative_time;
pub(crate) mod skeleton;
pub(crate) mod switcher;
pub(crate) mod tag;

pub use button::*;
pub use notification::{measure_notification_banner, render_notification_banner};
pub use relative_time::{format_rel, next_change_delay_ms};
pub use switcher::{SwitcherData, SwitcherTabData, switcher_size};
pub use tag::{TagTheme, tag_theme};

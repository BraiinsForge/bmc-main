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

//! Notification banner builder.

use crate::tree::Node;

/// Inline notification severity kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NotificationKind {
    Error = 0,
    Warning = 1,
    Success = 2,
    Info = 3,
}

/// Inline notification banner
pub fn notification(
    kind: NotificationKind,
    title: impl Into<String>,
    subtitle: impl Into<String>,
) -> Node {
    Node::Notification {
        kind,
        title: title.into(),
        subtitle: subtitle.into(),
    }
}

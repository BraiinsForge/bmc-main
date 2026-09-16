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

//! The typographic characters a widget sets, by name rather than by escape.
//!
//! `NBSP` and `DEGREE` double as what the host's number and temperature
//! formatting emit, which `bmc_shared_utils::typography` names on its side.
//! One definition cannot serve both: the SDK takes shared-utils on native
//! targets only, so formato never lands in a widget binary.

pub const NBSP: &str = "\u{a0}";
pub const ENDASH: &str = "\u{2013}";
pub const EMDASH: &str = "\u{2014}";
pub const ELLIPSIS: &str = "\u{2026}";
pub const LDQUO: &str = "\u{201c}";
pub const RDQUO: &str = "\u{201d}";
pub const TIMES: &str = "\u{d7}";
pub const DEGREE: &str = "\u{b0}";
pub const PRIME: &str = "\u{2032}";
pub const DOUBLE_PRIME: &str = "\u{2033}";
pub const BITCOIN: &str = "\u{20bf}";

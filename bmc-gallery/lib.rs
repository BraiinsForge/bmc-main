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

//! The gallery's scenes dylib: the discovered `*.scene.rs` compiled in,
//! plus the Deck kit they draw with (re-exported through [`prelude`]).

// Scenes are compiled in from wherever they sit beside their components, so
// `crate::` in one would read as the crate it lives next to. Name this one.
extern crate self as bmc_gallery;

pub mod kit;
pub mod prelude;

gallery::scenes_dylib!();

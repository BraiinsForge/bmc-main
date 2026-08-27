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

//! Shared machinery for widgets that display a picture fetched over HTTP.
//!
//! Three layers, none of which knows how or when the widget fetched:
//!
//! - [`machine`] — the pure view state machine. Its events are "a body
//!   arrived", "a decode finished", "the target changed"; never "a poll
//!   fired". No SDK calls, unit-testable on the host.
//! - [`render`] — the drawing surface: the fitted picture, status messages,
//!   the updating pill and the tap-to-reveal menu.
//! - [`picture`] — the decode → cache → restore path, as free functions that
//!   take the cache identity as an argument. Knows about slots, the flash
//!   cache and the host decoder; knows nothing about polls, params or URLs.
//!
//! Each widget keeps its own poll wiring, params reads,
//! [`machine::Action`] executor and `#[unsafe(no_mangle)]` exports:
//! every `Action` operates on a poll handle, and a widget may own two of them.
//!
//! `picture` is wasm32-only — it registers host assets. `machine` and `render`
//! build for the host too, so widget logic can be tested without a runtime.

pub mod machine;
#[cfg(target_arch = "wasm32")]
pub mod picture;
pub mod render;

pub use render::Fit;

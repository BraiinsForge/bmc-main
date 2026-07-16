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

// Typed TCP protocol for bmc-virt host↔guest IPC.
//
// The crate exposes two endpoint types:
// - `GuestEndpoint`: used by the relay daemon (guest side), accepts a connection
//   and provides typed send methods. Thread-safe via internal mpsc channel.
// - `HostEndpoint`: used by the console app (host side), connects to the relay
//   and provides a typed message iterator + input send method.
//
// All framing and serialization is internal — callers never touch raw TCP.

pub mod protocol;
pub mod types;

mod wire;

pub mod guest;
pub mod host;

pub use guest::{GuestConnection, GuestEndpoint, GuestSender};
pub use host::HostEndpoint;
pub use types::*;

#[cfg(test)]
mod tests;

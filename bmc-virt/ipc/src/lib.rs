// Copyright (C) 2026  Braiins Systems s.r.o.

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

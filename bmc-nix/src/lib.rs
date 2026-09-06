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

//! Nix package management for BMC.
//!
//! The `release-log-debug` feature removes TRACE instrumentation from
//! release builds while retaining DEBUG. `RUST_LOG=trace` cannot restore it.
//! Debug builds retain TRACE. The static CLI and Deck CLI packages enable
//! this feature; ordinary native development builds leave it disabled.
//!
//! Cargo unifies dependency features within a build graph: enabling this
//! feature in a workspace release build also caps other binaries in that build.
//! `bmc-openwrt` rejects builds that remove its TRACE instrumentation.

pub mod activation;
pub mod feed;
pub mod fs_sync;
pub mod gc;
pub mod generation_path;
pub mod hooks;
pub mod index;
pub mod installation;
pub mod manifest;
pub mod mount;
pub mod partition;
pub mod pending_install;
pub mod profile;
pub mod progress;
pub mod registration;
pub mod servers_config;
pub mod service_orchestrator;
pub mod signature;
pub mod store;
pub mod types;
pub mod upgrade;

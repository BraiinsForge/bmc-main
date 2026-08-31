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

//! Background host services used by the WASM runtime.

mod discovery;
mod fetch;
mod http;
mod socket;

pub(super) use discovery::{mdns_browse_thread, ssdp_search_thread, udp_broadcast_thread};
pub use fetch::FetchAgent;
pub(super) use fetch::{Redirects, do_fetch};
pub(super) use http::http_listener_thread;
pub(super) use socket::{
    TlsVerificationMode, host_tls_connect_impl, tcp_background_thread, ws_background_thread,
};

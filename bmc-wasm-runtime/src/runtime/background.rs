// Copyright (C) 2026  Braiins Systems s.r.o.

//! Background host services used by the WASM runtime.

mod discovery;
mod fetch;
mod http;
mod socket;

pub(super) use discovery::{mdns_browse_thread, ssdp_search_thread, udp_broadcast_thread};
pub(super) use fetch::do_fetch;
pub(super) use http::http_listener_thread;
pub(super) use socket::{
    TlsVerificationMode, host_tls_connect_impl, tcp_background_thread, ws_background_thread,
};

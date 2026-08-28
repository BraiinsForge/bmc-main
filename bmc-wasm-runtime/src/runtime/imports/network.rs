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

//! Network- and host-service-focused guest imports.

#![expect(clippy::cast_possible_truncation)]

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use bmc_wasm_protocol::{
    FetchOutcome, FetchRequestId, HttpListenerId, HttpRequestId, MdnsBrowseId, MdnsRegId,
    MediaTypePart, SocketId, SsdpSearchId, UdpBroadcastId, WebsocketId,
};
use wasmi::{Caller, Extern, Linker};

use crate::host_api::{
    ActiveHttpListener, ActiveMdnsBrowse, ActiveMdnsRegistration, ActiveSocket, ActiveSsdpSearch,
    ActiveUdpBroadcast, ActiveWebSocket, CancelDisposition, CompletedFetch, DelayedFetch,
    FetchCompletionContext, FetchRequestKey, HostState, HttpInboundRequest, HttpListenerResponse,
    MdnsEvent, SocketEvent, SocketOutbound, SsdpEvent, UdpBroadcastEvent, WsEvent, WsOutbound,
};

use super::super::background::{
    Redirects, TlsVerificationMode, do_fetch, host_tls_connect_impl, http_listener_thread,
    mdns_browse_thread, ssdp_search_thread, tcp_background_thread, udp_broadcast_thread,
    ws_background_thread,
};
use super::super::memory::{
    parse_headers, read_bytes, read_optional_bytes, read_string, write_to_wasm,
};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_fetch_now_import(linker)?;
    register_fetch_after_import(linker)?;
    register_fetch_body_ref_import(linker)?;
    register_fetch_cancel_import(linker)?;
    register_fetch_header_import(linker)?;
    register_fetch_media_type_import(linker)?;
    register_websocket_imports(linker)?;
    register_socket_connect_imports(linker)?;
    register_socket_io_imports(linker)?;
    register_mdns_browse_imports(linker)?;
    register_mdns_register_import(linker)?;
    register_mdns_unregister_import(linker)?;
    register_ssdp_imports(linker)?;
    register_udp_broadcast_imports(linker)?;
    register_http_listener_imports(linker)?;
    register_http_response_import(linker)?;
    register_network_info_import(linker)?;
    Ok(())
}

fn register_fetch_body_ref_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fetch_response_body_ref",
        |mut caller: Caller<'_, HostState>, request_id: u32| -> u32 {
            let Some(request_id) = FetchRequestId::from_wire(request_id) else {
                return 0;
            };
            let state = caller.data_mut();
            if !state.fetches.contains(request_id) {
                return 0;
            }
            state.fetch_body_refs.insert(request_id);
            1
        },
    )?;
    Ok(())
}

/// `host_network_info(out_ptr: *mut u8, out_cap: u32) -> u32`
/// — probe-then-allocate fetch of the encoded [`crate::network::NetworkInfo`]
/// (the Deck's SSID + IP). Same OOB-trap contract as `host_system_snapshot`:
/// `out_cap == 0` returns the required length;
/// `out_cap >= required` writes the bytes and returns how many.
fn register_network_info_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_network_info",
        |mut caller: Caller<'_, HostState>,
         out_ptr: u32,
         out_cap: u32|
         -> std::result::Result<u32, wasmi::Error> {
            let bytes = crate::network::encode(&caller.data().network_info);
            let needed = bytes.len() as u32;
            if out_cap < needed {
                return Ok(needed);
            }
            let Some(memory) = caller.get_export("memory").and_then(Extern::into_memory) else {
                return Err(wasmi::Error::new(
                    "host import `host_network_info`: guest module has no exported `memory` \
                     — cannot write the info. ABI requires an exported linear memory.",
                ));
            };
            let data = memory.data_mut(&mut caller);
            let start = out_ptr as usize;
            let end = start.saturating_add(bytes.len());
            if end > data.len() {
                return Err(wasmi::Error::new(format!(
                    "host import `host_network_info`: out_ptr range {start:#x}..{end:#x} \
                     overflows guest memory of {} bytes — size the buffer with the probe \
                     call (out_cap == 0) first",
                    data.len(),
                )));
            }
            data[start..end].copy_from_slice(&bytes);
            Ok(needed)
        },
    )?;
    Ok(())
}

fn register_fetch_now_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fetch",
        |mut caller: Caller<'_, HostState>,
         timeout_ms: u32,
         method_ptr: u32,
         method_len: u32,
         url_ptr: u32,
         url_len: u32,
         headers_ptr: u32,
         headers_len: u32,
         body_ptr: u32,
         body_len: u32|
         -> u32 {
            let Some(method) = read_string(&caller, method_ptr, method_len) else {
                return 0;
            };
            let Some(url) = read_string(&caller, url_ptr, url_len) else {
                return 0;
            };
            let headers = parse_headers(&caller, headers_ptr, headers_len);
            let body = read_optional_bytes(&caller, body_ptr, body_len);
            let timeout = Duration::from_millis(u64::from(timeout_ms));

            let state = caller.data_mut();
            if state.fetch_slots_used() >= state.resource_limits.max_fetches {
                tracing::warn!(
                    max_fetches = state.resource_limits.max_fetches,
                    "host_fetch rejected: runtime fetch limit reached"
                );
                return 0;
            }
            let request_id = FetchRequestId::alloc(&mut state.next_request_id);
            let key = FetchRequestKey::new(&method, &url);
            let settle = state.fetches.accept(request_id, key.clone());
            tracing::debug!(request_id = request_id.to_wire(), %method, %url, "starting HTTP fetch");

            let intercepted = state
                .fetch_interceptor
                .as_ref()
                .and_then(|f| f(&method, &url));
            if let Some(reply) = intercepted {
                let _ = settle.send(CompletedFetch::intercepted(request_id, reply));
                return request_id.to_wire();
            }

            if state.refuse_live_io("fetch", &key.joined()) {
                let _ = settle.send(CompletedFetch::host_decided(
                    request_id,
                    FetchOutcome::Network.to_wire(),
                    Vec::new(),
                    FetchCompletionContext::HermeticRefusal,
                ));
                return request_id.to_wire();
            }

            // Last hop before the wire. Everything above sees the placeholder
            // form — the fetch key, the interceptor, the hermetic-breach
            // record — so no diagnostic, fixture or log can hold a secret.
            let spent = match super::credentials::spend(state, &url, &headers, body) {
                Ok(spent) => spent,
                Err(refusal) => {
                    let _ = settle.send(CompletedFetch::host_decided(
                        request_id,
                        FetchOutcome::Refused.to_wire(),
                        Vec::new(),
                        FetchCompletionContext::CredentialRefusal(refusal),
                    ));
                    return request_id.to_wire();
                }
            };
            let super::credentials::SpentRequest {
                url,
                headers,
                body,
                carries_secret,
            } = spent;
            let redirects = Redirects::for_request(carries_secret);

            let tx = settle;
            let agent = state.fetch_agent.clone();
            std::thread::spawn(move || {
                let reply = do_fetch(
                    &agent,
                    &method,
                    &url,
                    &headers,
                    body.as_deref(),
                    timeout,
                    redirects,
                );
                let _ = tx.send(CompletedFetch {
                    request_id,
                    status: reply.status,
                    headers: reply.headers,
                    body: reply.body,
                    context: FetchCompletionContext::Normal,
                });
            });

            request_id.to_wire()
        },
    )?;

    Ok(())
}

fn register_fetch_after_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fetch_after",
        |mut caller: Caller<'_, HostState>,
         delay_ms: u32,
         timeout_ms: u32,
         method_ptr: u32,
         method_len: u32,
         url_ptr: u32,
         url_len: u32,
         headers_ptr: u32,
         headers_len: u32,
         body_ptr: u32,
         body_len: u32|
         -> u32 {
            let Some(method) = read_string(&caller, method_ptr, method_len) else {
                return 0;
            };
            let Some(url) = read_string(&caller, url_ptr, url_len) else {
                return 0;
            };
            let headers = parse_headers(&caller, headers_ptr, headers_len);
            let body = read_optional_bytes(&caller, body_ptr, body_len);
            let timeout = Duration::from_millis(u64::from(timeout_ms));

            let state = caller.data_mut();
            if state.fetch_slots_used() >= state.resource_limits.max_fetches {
                tracing::warn!(
                    max_fetches = state.resource_limits.max_fetches,
                    "host_fetch_after rejected: runtime fetch limit reached"
                );
                return 0;
            }
            let request_id = FetchRequestId::alloc(&mut state.next_request_id);

            let fire_at_ms = state.monotonic_ms + u64::from(delay_ms);
            state.fetches.queue_delayed(DelayedFetch {
                fire_at_ms,
                method,
                url,
                headers,
                body,
                timeout,
                request_id,
            });

            request_id.to_wire()
        },
    )?;

    Ok(())
}

fn register_fetch_cancel_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fetch_cancel",
        |mut caller: Caller<'_, HostState>, request_id: u32| -> u32 {
            let Some(request_id) = FetchRequestId::from_wire(request_id) else {
                return 0;
            };
            let state = caller.data_mut();
            match state.fetches.cancel(request_id) {
                CancelDisposition::Stopped => {
                    state.fetch_body_refs.remove(&request_id);
                    1
                }
                CancelDisposition::WillAbort => 0,
                CancelDisposition::Unknown => {
                    // An id below the counter was issued and settled; cancelling it
                    // again is an ordinary race. Only a fabricated id earns the line.
                    if request_id.to_wire() >= state.next_request_id {
                        tracing::warn!(
                            request_id = request_id.to_wire(),
                            "host_fetch_cancel ignored: no such request was ever issued"
                        );
                    }
                    0
                }
            }
        },
    )?;

    Ok(())
}

/// A header of the response being delivered right now.
///
/// Absent once delivery moves on, so a stashed request id reads nothing
/// rather than another response's headers.
/// Names compare case-insensitively, per RFC 9110 §5.1.
fn delivering_header(state: &HostState, request_id: FetchRequestId, name: &str) -> Option<String> {
    let (delivering, headers) = state.delivering_fetch_headers.as_ref()?;
    if *delivering != request_id {
        return None;
    }
    headers
        .iter()
        .find(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn register_fetch_header_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fetch_header",
        |mut caller: Caller<'_, HostState>,
         request_id: u32,
         name_ptr: u32,
         name_len: u32,
         out_ptr: u32,
         out_cap: u32|
         -> i32 {
            let Some(request_id) = FetchRequestId::from_wire(request_id) else {
                return -1;
            };
            let Some(name) = read_string(&caller, name_ptr, name_len) else {
                return -1;
            };
            let Some(value) = delivering_header(caller.data(), request_id, &name) else {
                return -1;
            };
            write_to_wasm(&mut caller, &value, out_ptr, out_cap)
        },
    )?;

    Ok(())
}

fn register_fetch_media_type_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fetch_media_type",
        |mut caller: Caller<'_, HostState>,
         request_id: u32,
         part: u32,
         out_ptr: u32,
         out_cap: u32|
         -> i32 {
            let Some(request_id) = FetchRequestId::from_wire(request_id) else {
                return -1;
            };
            let Some(part) = MediaTypePart::from_wire(part) else {
                return -1;
            };
            let Some(raw) = delivering_header(caller.data(), request_id, "content-type") else {
                return -1;
            };
            let Some(piece) = media_type_piece(&raw, part) else {
                return -1;
            };
            write_to_wasm(&mut caller, &piece, out_ptr, out_cap)
        },
    )?;

    Ok(())
}

/// One piece of a `Content-Type`, or `None` when the header does not parse
/// or carries no such piece. `mime` lowercases it, so callers need not.
fn media_type_piece(raw: &str, part: MediaTypePart) -> Option<String> {
    let media_type: mime::Mime = raw.parse().ok()?;
    // `mime` takes `application/` and reports an empty subtype
    // rather than refusing it; `""` would read as a subtype.
    if media_type.type_().as_str().is_empty() || media_type.subtype().as_str().is_empty() {
        return None;
    }
    Some(match part {
        MediaTypePart::Essence => media_type.essence_str().to_owned(),
        MediaTypePart::Type => media_type.type_().as_str().to_owned(),
        MediaTypePart::Subtype => media_type.subtype().as_str().to_owned(),
        // Absent rather than empty: `application/json` has no suffix at all.
        MediaTypePart::Suffix => media_type.suffix()?.as_str().to_owned(),
    })
}

fn register_websocket_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_ws_connect",
        |mut caller: Caller<'_, HostState>,
         url_ptr: u32,
         url_len: u32,
         headers_ptr: u32,
         headers_len: u32|
         -> u32 {
            let Some(url) = read_string(&caller, url_ptr, url_len) else {
                return 0;
            };
            let headers = parse_headers(&caller, headers_ptr, headers_len);

            let state = caller.data_mut();
            if state.websockets.len() >= state.resource_limits.max_websockets {
                tracing::warn!(
                    max_websockets = state.resource_limits.max_websockets,
                    "host_ws_connect rejected: runtime websocket limit reached"
                );
                return 0;
            }
            let ws_id = WebsocketId::alloc(&mut state.next_ws_id);

            let (event_tx, event_rx) = std::sync::mpsc::channel::<WsEvent>();

            if let Some(fixtures) = &mut state.event_fixtures {
                let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<WsOutbound>();
                state
                    .websockets
                    .insert(ws_id, ActiveWebSocket { msg_tx, event_rx });
                fixtures.ws_event_txs.insert(ws_id, event_tx);
            } else {
                let (msg_tx, msg_rx) = std::sync::mpsc::channel::<WsOutbound>();
                state
                    .websockets
                    .insert(ws_id, ActiveWebSocket { msg_tx, event_rx });
                if !state.refuse_live_io("websocket", &url) {
                    let ws_id_wire = ws_id.to_wire();
                    std::thread::spawn(move || {
                        ws_background_thread(ws_id_wire, &url, &headers, event_tx, msg_rx);
                    });
                }
            }

            ws_id.to_wire()
        },
    )?;

    linker.func_wrap(
        "env",
        "host_ws_send",
        |mut caller: Caller<'_, HostState>, ws_id: u32, msg_ptr: u32, msg_len: u32| -> u32 {
            let Some(msg) = read_string(&caller, msg_ptr, msg_len) else {
                return 1;
            };
            let Some(ws_id) = WebsocketId::from_wire(ws_id) else {
                return 1;
            };

            let state = caller.data_mut();
            let ok = state
                .websockets
                .get(&ws_id)
                .is_some_and(|ws| ws.msg_tx.send(WsOutbound::Text(msg)).is_ok());
            u32::from(!ok)
        },
    )?;

    linker.func_wrap(
        "env",
        "host_ws_close",
        |mut caller: Caller<'_, HostState>, ws_id: u32| {
            let Some(ws_id) = WebsocketId::from_wire(ws_id) else {
                return;
            };
            let state = caller.data_mut();
            if let Some(ws) = state.websockets.remove(&ws_id) {
                let _ = ws.msg_tx.send(WsOutbound::Close);
            }
        },
    )?;

    Ok(())
}

fn register_socket_connect_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_tcp_connect",
        |mut caller: Caller<'_, HostState>, host_ptr: u32, host_len: u32, port: u32| -> u32 {
            let Some(host) = read_string(&caller, host_ptr, host_len) else {
                return 0;
            };

            let state = caller.data_mut();
            if state.sockets.len() >= state.resource_limits.max_sockets {
                tracing::warn!(
                    max_sockets = state.resource_limits.max_sockets,
                    "host_tcp_connect rejected: runtime socket limit reached"
                );
                return 0;
            }
            let socket_id = SocketId::alloc(&mut state.next_socket_id);

            let (event_tx, event_rx) = std::sync::mpsc::channel::<SocketEvent>();

            if let Some(fixtures) = &mut state.event_fixtures {
                let (write_tx, _write_rx) = std::sync::mpsc::channel::<SocketOutbound>();
                state
                    .sockets
                    .insert(socket_id, ActiveSocket { write_tx, event_rx });
                fixtures.socket_event_txs.insert(socket_id, event_tx);
            } else {
                let (write_tx, write_rx) = std::sync::mpsc::channel::<SocketOutbound>();
                state
                    .sockets
                    .insert(socket_id, ActiveSocket { write_tx, event_rx });
                let port = port as u16;
                if !state.refuse_live_io("tcp", &format!("{host}:{port}")) {
                    let socket_id_wire = socket_id.to_wire();
                    std::thread::spawn(move || {
                        tcp_background_thread(socket_id_wire, &host, port, event_tx, write_rx);
                    });
                }
            }

            socket_id.to_wire()
        },
    )?;

    linker.func_wrap(
        "env",
        "host_tls_connect",
        |mut caller: Caller<'_, HostState>, host_ptr: u32, host_len: u32, port: u32| -> u32 {
            host_tls_connect_impl(
                &mut caller,
                host_ptr,
                host_len,
                port,
                TlsVerificationMode::Full,
            )
        },
    )?;

    linker.func_wrap(
        "env",
        "host_tls_connect_insecure",
        |mut caller: Caller<'_, HostState>, host_ptr: u32, host_len: u32, port: u32| -> u32 {
            host_tls_connect_impl(
                &mut caller,
                host_ptr,
                host_len,
                port,
                TlsVerificationMode::Insecure,
            )
        },
    )?;

    Ok(())
}

fn register_socket_io_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_socket_write",
        |mut caller: Caller<'_, HostState>, socket_id: u32, data_ptr: u32, data_len: u32| -> u32 {
            let Some(bytes) = read_bytes(&caller, data_ptr, data_len) else {
                return 1;
            };
            let Some(socket_id) = SocketId::from_wire(socket_id) else {
                return 1;
            };

            let state = caller.data_mut();
            let ok = state
                .sockets
                .get(&socket_id)
                .is_some_and(|s| s.write_tx.send(SocketOutbound::Data(bytes)).is_ok());
            u32::from(!ok)
        },
    )?;

    linker.func_wrap(
        "env",
        "host_socket_close",
        |mut caller: Caller<'_, HostState>, socket_id: u32| {
            let Some(socket_id) = SocketId::from_wire(socket_id) else {
                return;
            };
            let state = caller.data_mut();
            if let Some(sock) = state.sockets.remove(&socket_id) {
                let _ = sock.write_tx.send(SocketOutbound::Close);
            }
        },
    )?;

    Ok(())
}

fn register_mdns_browse_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_mdns_browse",
        |mut caller: Caller<'_, HostState>, svc_types_ptr: u32, svc_types_len: u32| -> u32 {
            let Some(raw) = read_string(&caller, svc_types_ptr, svc_types_len) else {
                return 0;
            };
            let service_types: Vec<String> = raw
                .lines()
                .map(|line| {
                    let line = line.trim();
                    if line.ends_with(".local.") {
                        line.to_owned()
                    } else {
                        format!("{line}.local.")
                    }
                })
                .filter(|svc| !svc.is_empty())
                .collect();
            if service_types.is_empty() {
                return 0;
            }

            let state = caller.data_mut();
            if state.mdns_browses.len() >= state.resource_limits.max_mdns_browses {
                tracing::warn!(
                    max_mdns_browses = state.resource_limits.max_mdns_browses,
                    "host_mdns_browse rejected: runtime mDNS browse limit reached"
                );
                return 0;
            }
            let browse_id = MdnsBrowseId::alloc(&mut state.next_mdns_browse_id);

            let (event_tx, event_rx) = std::sync::mpsc::channel::<MdnsEvent>();
            let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

            state
                .mdns_browses
                .insert(browse_id, ActiveMdnsBrowse { event_rx, stop_tx });

            if let Some(ref mut ef) = state.event_fixtures {
                drop(stop_rx);
                ef.mdns_event_txs.insert(browse_id, event_tx);
            } else if !state.refuse_live_io("mdns-browse", &service_types.join(", ")) {
                std::thread::spawn(move || {
                    mdns_browse_thread(service_types, event_tx, stop_rx);
                });
            }

            browse_id.to_wire()
        },
    )?;

    linker.func_wrap(
        "env",
        "host_mdns_stop",
        |mut caller: Caller<'_, HostState>, browse_id: u32| {
            let Some(browse_id) = MdnsBrowseId::from_wire(browse_id) else {
                return;
            };
            let state = caller.data_mut();
            if let Some(browse) = state.mdns_browses.remove(&browse_id) {
                let _ = browse.stop_tx.send(());
            }
        },
    )?;

    Ok(())
}

fn register_mdns_register_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_mdns_register",
        |mut caller: Caller<'_, HostState>,
         svc_ptr: u32,
         svc_len: u32,
         name_ptr: u32,
         name_len: u32,
         port: u32,
         txt_ptr: u32,
         txt_len: u32|
         -> u32 {
            let svc_type = read_string(&caller, svc_ptr, svc_len);
            let name = read_string(&caller, name_ptr, name_len);
            let txt_raw = if txt_len > 0 {
                read_string(&caller, txt_ptr, txt_len)
            } else {
                Some(String::new())
            };
            let (Some(svc_type), Some(name), Some(txt_raw)) = (svc_type, name, txt_raw) else {
                return 0;
            };

            if caller.data().mdns_registrations.len()
                >= caller.data().resource_limits.max_mdns_registrations
            {
                tracing::warn!(
                    max_mdns_registrations = caller.data().resource_limits.max_mdns_registrations,
                    "host_mdns_register rejected: runtime mDNS registration limit reached"
                );
                return 0;
            }

            let svc_type = if svc_type.ends_with(".local.") {
                svc_type
            } else {
                format!("{svc_type}.local.")
            };

            let port = port as u16;
            let properties: Vec<(String, String)> = txt_raw
                .lines()
                .filter_map(|line| {
                    line.split_once('=')
                        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                })
                .collect();

            if caller.data_mut().refuse_live_io("mdns-register", &svc_type) {
                return 0;
            }

            let daemon = match mdns_sd::ServiceDaemon::new() {
                Ok(daemon) => daemon,
                Err(e) => {
                    tracing::error!("mDNS daemon creation failed: {e}");
                    return 0;
                }
            };

            let hostname = format!("{}.local.", name.replace(' ', "-"));
            let txt: HashMap<String, String> = properties.into_iter().collect();
            let info = match mdns_sd::ServiceInfo::new(&svc_type, &name, &hostname, "", port, txt) {
                Ok(info) => info,
                Err(e) => {
                    tracing::error!("mDNS ServiceInfo creation failed: {e}");
                    return 0;
                }
            };
            let fullname = info.get_fullname().to_owned();

            if let Err(e) = daemon.register(info) {
                tracing::error!("mDNS register failed: {e}");
                return 0;
            }

            let state = caller.data_mut();
            let reg_id = MdnsRegId::alloc(&mut state.next_mdns_reg_id);
            state
                .mdns_registrations
                .insert(reg_id, ActiveMdnsRegistration { daemon, fullname });

            reg_id.to_wire()
        },
    )?;

    Ok(())
}

fn register_mdns_unregister_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_mdns_unregister",
        |mut caller: Caller<'_, HostState>, reg_id: u32| {
            let Some(reg_id) = MdnsRegId::from_wire(reg_id) else {
                return;
            };
            let state = caller.data_mut();
            if let Some(reg) = state.mdns_registrations.remove(&reg_id) {
                let _ = reg.daemon.unregister(&reg.fullname);
                let _ = reg.daemon.shutdown();
            }
        },
    )?;

    Ok(())
}

fn register_ssdp_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_ssdp_search",
        |mut caller: Caller<'_, HostState>, st_ptr: u32, st_len: u32, timeout_secs: u32| -> u32 {
            let Some(search_target) = read_string(&caller, st_ptr, st_len) else {
                return 0;
            };
            if search_target.is_empty() {
                return 0;
            }

            let state = caller.data_mut();
            if state.ssdp_searches.len() >= state.resource_limits.max_ssdp_searches {
                tracing::warn!(
                    max_ssdp_searches = state.resource_limits.max_ssdp_searches,
                    "host_ssdp_search rejected: runtime SSDP search limit reached"
                );
                return 0;
            }
            let search_id = SsdpSearchId::alloc(&mut state.next_ssdp_search_id);

            let (event_tx, event_rx) = std::sync::mpsc::channel::<SsdpEvent>();
            let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

            state
                .ssdp_searches
                .insert(search_id, ActiveSsdpSearch { event_rx, stop_tx });

            if let Some(ref mut ef) = state.event_fixtures {
                drop(stop_rx);
                ef.ssdp_event_txs.insert(search_id, event_tx);
            } else if !state.refuse_live_io("ssdp", &search_target) {
                std::thread::spawn(move || {
                    ssdp_search_thread(search_target, timeout_secs, event_tx, stop_rx);
                });
            }

            search_id.to_wire()
        },
    )?;

    linker.func_wrap(
        "env",
        "host_ssdp_stop",
        |mut caller: Caller<'_, HostState>, search_id: u32| {
            let Some(search_id) = SsdpSearchId::from_wire(search_id) else {
                return;
            };
            let state = caller.data_mut();
            if let Some(search) = state.ssdp_searches.remove(&search_id) {
                let _ = search.stop_tx.send(());
            }
        },
    )?;

    Ok(())
}

fn register_udp_broadcast_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_udp_broadcast",
        |mut caller: Caller<'_, HostState>,
         port: u32,
         msg_ptr: u32,
         msg_len: u32,
         timeout_secs: u32|
         -> u32 {
            let Some(message) = read_string(&caller, msg_ptr, msg_len) else {
                return 0;
            };
            if message.is_empty() {
                return 0;
            }

            let state = caller.data_mut();
            if state.udp_broadcasts.len() >= state.resource_limits.max_udp_broadcasts {
                tracing::warn!(
                    max_udp_broadcasts = state.resource_limits.max_udp_broadcasts,
                    "host_udp_broadcast rejected: runtime UDP broadcast limit reached"
                );
                return 0;
            }
            let broadcast_id = UdpBroadcastId::alloc(&mut state.next_udp_broadcast_id);

            let (event_tx, event_rx) = std::sync::mpsc::channel::<UdpBroadcastEvent>();
            let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

            state
                .udp_broadcasts
                .insert(broadcast_id, ActiveUdpBroadcast { event_rx, stop_tx });

            if let Some(ref mut ef) = state.event_fixtures {
                drop(stop_rx);
                ef.udp_event_txs.insert(broadcast_id, event_tx);
            } else if !state.refuse_live_io("udp", &format!("port {port}")) {
                std::thread::spawn(move || {
                    udp_broadcast_thread(port, message, timeout_secs, event_tx, stop_rx);
                });
            }

            broadcast_id.to_wire()
        },
    )?;

    linker.func_wrap(
        "env",
        "host_udp_broadcast_stop",
        |mut caller: Caller<'_, HostState>, broadcast_id: u32| {
            let Some(broadcast_id) = UdpBroadcastId::from_wire(broadcast_id) else {
                return;
            };
            let state = caller.data_mut();
            if let Some(broadcast) = state.udp_broadcasts.remove(&broadcast_id) {
                let _ = broadcast.stop_tx.send(());
            }
        },
    )?;

    Ok(())
}

fn register_http_listener_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_http_listen",
        |mut caller: Caller<'_, HostState>, port: u32| -> u32 {
            let port = port as u16;

            let state = caller.data_mut();
            if state.http_listeners.len() >= state.resource_limits.max_http_listeners {
                tracing::warn!(
                    max_http_listeners = state.resource_limits.max_http_listeners,
                    "host_http_listen rejected: runtime HTTP listener limit reached"
                );
                return 0;
            }
            if state.refuse_live_io("http-listen", &format!("port {port}")) {
                return 0;
            }
            let listener_id = HttpListenerId::alloc(&mut state.next_http_listener_id);

            let (request_tx, request_rx) = std::sync::mpsc::channel::<HttpInboundRequest>();
            let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
            let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();

            std::thread::spawn(move || {
                http_listener_thread(port, request_tx, stop_rx, port_tx);
            });

            let actual_port = port_rx.recv_timeout(Duration::from_secs(2)).unwrap_or(port);

            state.http_listeners.insert(
                listener_id,
                ActiveHttpListener {
                    request_rx,
                    stop_tx,
                    port: actual_port,
                },
            );

            listener_id.to_wire()
        },
    )?;

    linker.func_wrap(
        "env",
        "host_http_close_listener",
        |mut caller: Caller<'_, HostState>, listener_id: u32| {
            let Some(listener_id) = HttpListenerId::from_wire(listener_id) else {
                return;
            };
            let state = caller.data_mut();
            if let Some(listener) = state.http_listeners.remove(&listener_id) {
                let _ = listener.stop_tx.send(());
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "host_http_get_port",
        |caller: Caller<'_, HostState>, listener_id: u32| -> u32 {
            let Some(listener_id) = HttpListenerId::from_wire(listener_id) else {
                return 0;
            };
            let state = caller.data();
            state
                .http_listeners
                .get(&listener_id)
                .map_or(0, |listener| u32::from(listener.port))
        },
    )?;

    Ok(())
}

fn register_http_response_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_http_respond",
        |mut caller: Caller<'_, HostState>,
         request_id: u32,
         status: u32,
         headers_ptr: u32,
         headers_len: u32,
         body_ptr: u32,
         body_len: u32| {
            let headers = if headers_len > 0 {
                read_string(&caller, headers_ptr, headers_len).unwrap_or_default()
            } else {
                String::new()
            };
            let body = if body_len > 0 {
                read_bytes(&caller, body_ptr, body_len).unwrap_or_default()
            } else {
                Vec::new()
            };

            let Some(request_id) = HttpRequestId::from_wire(request_id) else {
                return;
            };
            let state = caller.data_mut();
            if let Some(tx) = state.http_response_txs.remove(&request_id) {
                let _ = tx.send(HttpListenerResponse {
                    status: status as u16,
                    headers,
                    body,
                });
            }
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::MediaTypePart;

    use super::{delivering_header, media_type_piece};
    use crate::host_api::HostState;
    use crate::runtime_limits::RuntimeResourceLimits;

    fn part(raw: &str, part: MediaTypePart) -> Option<String> {
        media_type_piece(raw, part)
    }

    fn subtype(raw: &str) -> Option<String> {
        part(raw, MediaTypePart::Subtype)
    }

    fn suffix(raw: &str) -> Option<String> {
        part(raw, MediaTypePart::Suffix)
    }

    /// The widget's own JSON gate, checked against the pieces it reads.
    fn names_json(raw: &str) -> bool {
        subtype(raw).as_deref() == Some("json") || suffix(raw).as_deref() == Some("json")
    }

    #[test]
    fn splits_a_plain_media_type() {
        assert_eq!(
            part("application/json", MediaTypePart::Essence).as_deref(),
            Some("application/json")
        );
        assert_eq!(
            part("application/json", MediaTypePart::Type).as_deref(),
            Some("application")
        );
        assert_eq!(subtype("application/json").as_deref(), Some("json"));
        assert_eq!(
            suffix("application/json"),
            None,
            "a subtype with no `+` has no suffix, which is not the same as an empty one"
        );
    }

    #[test]
    fn normalises_case_and_drops_parameters() {
        assert_eq!(
            part("Application/JSON; charset=utf-8", MediaTypePart::Essence).as_deref(),
            Some("application/json"),
            "the essence is the type alone, lowercased"
        );
        assert_eq!(subtype("APPLICATION/JSON").as_deref(), Some("json"));
    }

    #[test]
    fn reads_an_rfc_6839_suffix() {
        assert_eq!(
            subtype("application/vnd.api+json").as_deref(),
            Some("vnd.api")
        );
        assert_eq!(suffix("application/vnd.api+json").as_deref(), Some("json"));
    }

    #[test]
    fn only_a_media_type_that_names_json_passes() {
        for raw in [
            "application/json",
            "Application/JSON; charset=utf-8",
            "application/vnd.api+json",
            "text/json",
        ] {
            assert!(names_json(raw), "{raw} names JSON");
        }
        for raw in [
            "application/notjson",
            "application/json-seq",
            "text/html; profile=json",
            "text/html",
            "application/xml",
        ] {
            assert!(!names_json(raw), "{raw} does not name JSON");
        }
    }

    #[test]
    fn a_header_that_does_not_parse_yields_nothing() {
        for raw in [
            "",
            "   ",
            "json",
            "application",
            "application/",
            "/json",
            "application//json",
            "application/json/extra",
            "app lication/json",
            "application/js on",
            ";charset=utf-8",
            "application/json; charset",
        ] {
            assert_eq!(subtype(raw), None, "{raw:?} must not parse to a subtype");
            assert!(!names_json(raw), "{raw:?} must not read as JSON");
        }
    }

    #[test]
    fn a_header_is_found_whatever_its_case() {
        let mut state = HostState::new(
            RuntimeResourceLimits::default(),
            chrono::Local::now().fixed_offset(),
        );
        let id = bmc_wasm_protocol::FetchRequestId::from_wire(1).expect("BUG: 1 is a valid id");
        state.delivering_fetch_headers = Some((
            id,
            vec![("content-type".to_owned(), "application/json".to_owned())],
        ));

        for name in ["content-type", "Content-Type", "CONTENT-TYPE"] {
            assert_eq!(
                delivering_header(&state, id, name).as_deref(),
                Some("application/json"),
                "{name} must find the header RFC 9110 says is the same one"
            );
        }
        assert_eq!(delivering_header(&state, id, "retry-after"), None);
    }

    #[test]
    fn another_requests_headers_are_never_returned() {
        let mut state = HostState::new(
            RuntimeResourceLimits::default(),
            chrono::Local::now().fixed_offset(),
        );
        let delivering =
            bmc_wasm_protocol::FetchRequestId::from_wire(1).expect("BUG: 1 is a valid id");
        let other = bmc_wasm_protocol::FetchRequestId::from_wire(2).expect("BUG: 2 is a valid id");
        state.delivering_fetch_headers = Some((
            delivering,
            vec![("content-type".to_owned(), "application/json".to_owned())],
        ));

        assert_eq!(
            delivering_header(&state, other, "content-type"),
            None,
            "a stashed id must not read the response being delivered now"
        );

        state.delivering_fetch_headers = None;
        assert_eq!(
            delivering_header(&state, delivering, "content-type"),
            None,
            "headers are unreadable once delivery has moved on"
        );
    }
}

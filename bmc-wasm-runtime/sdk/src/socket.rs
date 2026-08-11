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

//! TLS socket support for WASM widgets.
//!
//! Provides `tls_connect()` for verified TLS connections and
//! `tls_connect_insecure()` for trusted self-signed LAN devices. The host
//! manages the TCP+TLS I/O in a background thread and delivers events by
//! calling the `__on_socket_event` export.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::*;
//! use bmc_wasm_sdk::socket::*;
//!
//! fn init(width: u32, height: u32) {
//!     tls_connect("192.168.1.50", 8009, on_socket_event);
//! }
//!
//! fn on_socket_event(socket: Socket, event: &SocketEvent<'_>) {
//!     match event {
//!         SocketEvent::Connected => {
//!             log_info!("connected!");
//!             socket.write(b"hello");
//!         }
//!         SocketEvent::Data(data) => {
//!             log_info!("received {} bytes", data.len());
//!         }
//!         SocketEvent::Closed(code) => {
//!             log_info!("closed with code {code}");
//!         }
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_protocol::SocketId;

// Host function imports
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_tls_connect(host_ptr: *const u8, host_len: u32, port: u32) -> u32;
    fn host_tls_connect_insecure(host_ptr: *const u8, host_len: u32, port: u32) -> u32;
    fn host_tcp_connect(host_ptr: *const u8, host_len: u32, port: u32) -> u32;
    fn host_socket_write(socket_id: u32, data_ptr: *const u8, data_len: u32) -> u32;
    fn host_socket_close(socket_id: u32);
}

/// Callback type: `fn(socket, event)`.
pub type Callback = fn(Socket, &SocketEvent<'_>);

/// Handle to an active TLS socket connection.
#[derive(Clone, Copy, Debug)]
pub struct Socket(pub SocketId);

impl Socket {
    /// Write data bytes to the socket.
    pub fn write(&self, data: &[u8]) {
        unsafe {
            host_socket_write(self.0.to_wire(), data.as_ptr(), data.len() as u32);
        }
    }

    /// Close the socket connection.
    pub fn close(&self) {
        unsafe {
            host_socket_close(self.0.to_wire());
        }
    }
}

/// Event delivered from the host for a TLS socket.
#[derive(Debug)]
pub enum SocketEvent<'a> {
    /// Connection successfully established.
    Connected,
    /// Data received from the remote end.
    Data(&'a [u8]),
    /// Connection closed (0 = normal, non-zero = error).
    Closed(u32),
}

thread_local! {
    static CALLBACKS: RefCell<Vec<Callback>> = const { RefCell::new(Vec::new()) };
    static CONNECTIONS: RefCell<HashMap<SocketId, usize>> = RefCell::new(HashMap::new());
}

fn register_callback(cb: Callback) -> usize {
    CALLBACKS.with(|cbs| {
        let mut cbs = cbs.borrow_mut();
        for (i, existing) in cbs.iter().enumerate() {
            if *existing as usize == cb as usize {
                return i;
            }
        }
        let idx = cbs.len();
        cbs.push(cb);
        idx
    })
}

/// Connect to a TLS socket. The host performs the TCP+TLS handshake in the
/// background. When events arrive (connected, data, closed), `callback` is
/// called. Certificate verification is enabled.
///
/// Returns `None` if the host rejects the connection before it is queued.
#[must_use]
pub fn tls_connect(host: &str, port: u16, callback: Callback) -> Option<Socket> {
    let cb_idx = register_callback(callback);
    let socket_id = SocketId::from_wire(unsafe {
        host_tls_connect(host.as_ptr(), host.len() as u32, u32::from(port))
    })?;
    CONNECTIONS.with(|c| c.borrow_mut().insert(socket_id, cb_idx));
    Some(Socket(socket_id))
}

/// Connect to a TLS socket while skipping certificate verification.
///
/// Use this only for trusted LAN devices with self-signed certificates such as
/// Chromecast receivers.
///
/// Returns `None` if the host rejects the connection before it is queued.
#[must_use]
pub fn tls_connect_insecure(host: &str, port: u16, callback: Callback) -> Option<Socket> {
    let cb_idx = register_callback(callback);
    let socket_id = SocketId::from_wire(unsafe {
        host_tls_connect_insecure(host.as_ptr(), host.len() as u32, u32::from(port))
    })?;
    CONNECTIONS.with(|c| c.borrow_mut().insert(socket_id, cb_idx));
    Some(Socket(socket_id))
}

/// Connect to a plain TCP socket. The host performs the TCP connect in the
/// background. When events arrive (connected, data, closed), `callback` is
/// called.
///
/// Returns `None` if the host rejects the connection before it is queued.
#[must_use]
pub fn tcp_connect(host: &str, port: u16, callback: Callback) -> Option<Socket> {
    let cb_idx = register_callback(callback);
    let socket_id = SocketId::from_wire(unsafe {
        host_tcp_connect(host.as_ptr(), host.len() as u32, u32::from(port))
    })?;
    CONNECTIONS.with(|c| c.borrow_mut().insert(socket_id, cb_idx));
    Some(Socket(socket_id))
}

/// Called by the host when a socket event is ready.
#[unsafe(no_mangle)]
pub extern "C" fn __on_socket_event(socket_id: u32, event_type: u32, data_ptr: u32, data_len: u32) {
    let Some(socket_id) = SocketId::from_wire(socket_id) else {
        return;
    };

    let data = if data_len > 0 && data_ptr != 0 {
        unsafe { Vec::from_raw_parts(data_ptr as *mut u8, data_len as usize, data_len as usize) }
    } else {
        Vec::new()
    };

    let event = match event_type {
        0 => SocketEvent::Connected,
        1 => SocketEvent::Data(&data),
        2 => {
            let code = if data.len() >= 4 {
                u32::from_le_bytes([data[0], data[1], data[2], data[3]])
            } else {
                1
            };
            SocketEvent::Closed(code)
        }
        _ => return,
    };

    let is_close = matches!(event, SocketEvent::Closed(_));
    let socket = Socket(socket_id);

    let cb = CONNECTIONS
        .with(|c| {
            if is_close {
                c.borrow_mut().remove(&socket_id)
            } else {
                c.borrow().get(&socket_id).copied()
            }
        })
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        cb(socket, &event);
    }
}

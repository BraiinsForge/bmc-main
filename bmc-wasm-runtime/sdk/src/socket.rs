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
//! A [`Socket`] is the only handle to its connection,
//! and dropping it closes the connection,
//! so a widget keeps the handle for as long as it wants the connection.
//! The callback borrows the socket its event belongs to;
//! to close the connection from there, take the stored handle.
//! When the peer closes the connection, the callback gets [`SocketEvent::Closed`]
//! and the stored handle reaches nothing, so clear or replace it there.
//!
//! Off-target there is no host to connect through, so connecting or writing panics.
//!
//! # Example
//!
//! ```ignore
//! use std::cell::RefCell;
//!
//! use bmc_wasm_sdk::*;
//! use bmc_wasm_sdk::socket::*;
//!
//! thread_local! {
//!     static SOCKET: RefCell<Option<Socket>> = const { RefCell::new(None) };
//! }
//!
//! #[unsafe(no_mangle)]
//! pub extern "C" fn init() {
//!     SOCKET.set(tls_connect("192.168.1.50", 8009, on_socket_event));
//! }
//!
//! fn on_socket_event(socket: &Socket, event: &SocketEvent<'_>) {
//!     match event {
//!         SocketEvent::Connected => socket.write(b"hello"),
//!         SocketEvent::Data(b"bye") => {
//!             if let Some(socket) = SOCKET.take() {
//!                 socket.close();
//!             }
//!         }
//!         SocketEvent::Data(data) => log_info!("received {} bytes", data.len()),
//!         SocketEvent::Closed(code) => {
//!             log_info!("closed with code {code}");
//!             SOCKET.set(None);
//!         }
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use bmc_wasm_protocol::SocketId;

/// Callback type: `fn(socket, event)`.
pub type Callback = fn(&Socket, &SocketEvent<'_>);

/// The only handle to a socket connection; dropping it closes the connection.
#[derive(Debug, PartialEq, Eq)]
pub struct Socket(SocketId);

impl Socket {
    /// Write data bytes to the socket.
    ///
    /// Does nothing once the connection has closed,
    /// such as on a callback's argument after the callback closed the stored handle.
    pub fn write(&self, data: &[u8]) {
        Platform::write(self.0, data);
    }

    /// Close the connection. An event already due in the same delivery pass is dropped.
    pub fn close(self) {
        drop(self);
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        CONNECTIONS.with_borrow_mut(|connections| connections.remove(&self.0));
        Platform::close(self.0);
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
    ///
    /// The widget's stored handle reaches nothing from here on; clear or replace it.
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
/// Returns `None` if the host refuses the connection before it is queued.
#[must_use = "dropping the returned Socket closes it"]
pub fn tls_connect(host: &str, port: u16, callback: Callback) -> Option<Socket> {
    let socket_id = Platform::tls_connect(host, port)?;
    let cb_idx = register_callback(callback);
    CONNECTIONS.with(|c| c.borrow_mut().insert(socket_id, cb_idx));
    Some(Socket(socket_id))
}

/// Connect to a TLS socket while skipping certificate verification.
///
/// Use this only for trusted LAN devices with self-signed certificates such as
/// Chromecast receivers.
///
/// Returns `None` if the host refuses the connection before it is queued.
#[must_use = "dropping the returned Socket closes it"]
pub fn tls_connect_insecure(host: &str, port: u16, callback: Callback) -> Option<Socket> {
    let socket_id = Platform::tls_connect_insecure(host, port)?;
    let cb_idx = register_callback(callback);
    CONNECTIONS.with(|c| c.borrow_mut().insert(socket_id, cb_idx));
    Some(Socket(socket_id))
}

/// Connect to a plain TCP socket. The host performs the TCP connect in the
/// background. When events arrive (connected, data, closed), `callback` is
/// called.
///
/// Returns `None` if the host refuses the connection before it is queued.
#[must_use = "dropping the returned Socket closes it"]
pub fn tcp_connect(host: &str, port: u16, callback: Callback) -> Option<Socket> {
    let socket_id = Platform::tcp_connect(host, port)?;
    let cb_idx = register_callback(callback);
    CONNECTIONS.with(|c| c.borrow_mut().insert(socket_id, cb_idx));
    Some(Socket(socket_id))
}

/// Hand an event to the callback of its socket, if a handle still holds it.
///
/// A close unregisters the socket before the callback runs.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "off-target no host delivers events")
)]
fn deliver(socket_id: SocketId, event: &SocketEvent<'_>) {
    let is_close = matches!(event, SocketEvent::Closed(_));

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
        // Borrowed for the event only; dropping it would close the connection.
        cb(&ManuallyDrop::new(Socket(socket_id)), event);
    }
}

trait SocketTransport {
    fn tls_connect(host: &str, port: u16) -> Option<SocketId>;
    fn tls_connect_insecure(host: &str, port: u16) -> Option<SocketId>;
    fn tcp_connect(host: &str, port: u16) -> Option<SocketId>;
    fn write(socket_id: SocketId, data: &[u8]);
    fn close(socket_id: SocketId);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_wasm_protocol::SocketId;

    use super::{SocketEvent, deliver};

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_tls_connect(host_ptr: *const u8, host_len: u32, port: u32) -> u32;
        fn host_tls_connect_insecure(host_ptr: *const u8, host_len: u32, port: u32) -> u32;
        fn host_tcp_connect(host_ptr: *const u8, host_len: u32, port: u32) -> u32;
        fn host_socket_write(socket_id: u32, data_ptr: *const u8, data_len: u32) -> u32;
        fn host_socket_close(socket_id: u32);
    }

    pub(super) struct Host;

    impl super::SocketTransport for Host {
        fn tls_connect(host: &str, port: u16) -> Option<SocketId> {
            SocketId::from_wire(unsafe {
                host_tls_connect(host.as_ptr(), crate::ffi_u32(host.len()), u32::from(port))
            })
        }

        fn tls_connect_insecure(host: &str, port: u16) -> Option<SocketId> {
            SocketId::from_wire(unsafe {
                host_tls_connect_insecure(
                    host.as_ptr(),
                    crate::ffi_u32(host.len()),
                    u32::from(port),
                )
            })
        }

        fn tcp_connect(host: &str, port: u16) -> Option<SocketId> {
            SocketId::from_wire(unsafe {
                host_tcp_connect(host.as_ptr(), crate::ffi_u32(host.len()), u32::from(port))
            })
        }

        fn write(socket_id: SocketId, data: &[u8]) {
            unsafe {
                host_socket_write(
                    socket_id.to_wire(),
                    data.as_ptr(),
                    crate::ffi_u32(data.len()),
                );
            }
        }

        fn close(socket_id: SocketId) {
            unsafe {
                host_socket_close(socket_id.to_wire());
            }
        }
    }

    /// Called by the host when a socket event is ready.
    #[expect(
        clippy::same_length_and_capacity,
        reason = "the host hands back the size it asked __alloc for"
    )]
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_socket_event(
        socket_id: u32,
        event_type: u32,
        data_ptr: u32,
        data_len: u32,
    ) {
        let Some(socket_id) = SocketId::from_wire(socket_id) else {
            return;
        };

        let data = if data_len > 0 && data_ptr != 0 {
            unsafe {
                Vec::from_raw_parts(data_ptr as *mut u8, data_len as usize, data_len as usize)
            }
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

        deliver(socket_id, &event);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_wasm_protocol::SocketId;

    pub(super) struct Host;

    impl super::SocketTransport for Host {
        fn tls_connect(_: &str, _: u16) -> Option<SocketId> {
            panic!("BUG: tls_connect() called off-target — no host to connect through")
        }

        fn tls_connect_insecure(_: &str, _: u16) -> Option<SocketId> {
            panic!("BUG: tls_connect_insecure() called off-target — no host to connect through")
        }

        fn tcp_connect(_: &str, _: u16) -> Option<SocketId> {
            panic!("BUG: tcp_connect() called off-target — no host to connect through")
        }

        fn write(_: SocketId, _: &[u8]) {
            panic!("BUG: Socket::write() called off-target — no host to connect through")
        }

        /// Only a test holds a handle off-target, and there is no host connection to close.
        fn close(_: SocketId) {}
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use bmc_wasm_protocol::SocketId;

    use super::{CONNECTIONS, Callback, Socket, SocketEvent, register_callback};
    use crate::wasm_test;

    thread_local! {
        static SEEN: Cell<Option<SocketId>> = const { Cell::new(None) };
        static STORED: RefCell<Option<Socket>> = const { RefCell::new(None) };
    }

    fn connect(callback: Callback) -> Socket {
        let id = SocketId::from_wire(1).expect("BUG: test ids are non-zero");
        let cb_idx = register_callback(callback);
        CONNECTIONS.with_borrow_mut(|connections| connections.insert(id, cb_idx));
        Socket(id)
    }

    fn deliver(id: SocketId) {
        super::deliver(id, &SocketEvent::Connected);
    }

    fn is_registered(id: SocketId) -> bool {
        CONNECTIONS.with_borrow(|registered| registered.contains_key(&id))
    }

    fn record(socket: &Socket, _event: &SocketEvent<'_>) {
        SEEN.set(Some(socket.0));
    }

    fn close_stored(_socket: &Socket, _event: &SocketEvent<'_>) {
        if let Some(stored) = STORED.take() {
            stored.close();
        }
    }

    #[wasm_test]
    fn an_event_reaches_its_callback_with_its_socket() {
        let socket = connect(record);

        deliver(socket.0);

        assert_eq!(SEEN.get(), Some(socket.0));
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_test]
    fn a_connect_from_the_host_reaches_its_socket() {
        let socket = connect(record);

        super::wasm::__on_socket_event(socket.0.to_wire(), 0, 0, 0);

        assert_eq!(SEEN.get(), Some(socket.0));
    }

    #[wasm_test]
    fn an_event_leaves_its_socket_open() {
        let socket = connect(record);

        deliver(socket.0);

        assert!(is_registered(socket.0));
    }

    #[wasm_test]
    fn a_dropped_handle_stops_its_events() {
        let socket = connect(record);
        let id = socket.0;

        drop(socket);
        deliver(id);

        assert_eq!(SEEN.get(), None);
    }

    #[wasm_test]
    fn a_callback_can_close_its_own_socket_through_the_stored_handle() {
        let socket = connect(close_stored);
        let id = socket.0;
        STORED.set(Some(socket));

        deliver(id);

        assert!(!is_registered(id));
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! TLS socket support for WASM widgets.
//!
//! Provides `tls_connect()` for establishing TLS connections. The host manages
//! the TCP+TLS I/O in a background thread and delivers events by calling the
//! `__on_socket_event` export.
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

// Host function imports
unsafe extern "C" {
    fn host_tls_connect(host_ptr: *const u8, host_len: u32, port: u32) -> u32;
    fn host_tcp_connect(host_ptr: *const u8, host_len: u32, port: u32) -> u32;
    fn host_socket_write(socket_id: u32, data_ptr: *const u8, data_len: u32) -> u32;
    fn host_socket_close(socket_id: u32);
}

/// Callback type: `fn(socket, event)`.
pub type Callback = fn(Socket, &SocketEvent<'_>);

/// Handle to an active TLS socket connection.
#[derive(Clone, Copy)]
pub struct Socket(pub u32);

impl Socket {
    /// Write data bytes to the socket.
    pub fn write(&self, data: &[u8]) {
        unsafe {
            host_socket_write(self.0, data.as_ptr(), data.len() as u32);
        }
    }

    /// Close the socket connection.
    pub fn close(&self) {
        unsafe {
            host_socket_close(self.0);
        }
    }
}

/// Event delivered from the host for a TLS socket.
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
    static CONNECTIONS: RefCell<HashMap<u32, usize>> = RefCell::new(HashMap::new());
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
/// called.
pub fn tls_connect(host: &str, port: u16, callback: Callback) -> Socket {
    let cb_idx = register_callback(callback);
    let socket_id = unsafe { host_tls_connect(host.as_ptr(), host.len() as u32, u32::from(port)) };
    CONNECTIONS.with(|c| c.borrow_mut().insert(socket_id, cb_idx));
    Socket(socket_id)
}

/// Connect to a plain TCP socket. The host performs the TCP connect in the
/// background. When events arrive (connected, data, closed), `callback` is
/// called.
pub fn tcp_connect(host: &str, port: u16, callback: Callback) -> Socket {
    let cb_idx = register_callback(callback);
    let socket_id = unsafe { host_tcp_connect(host.as_ptr(), host.len() as u32, u32::from(port)) };
    CONNECTIONS.with(|c| c.borrow_mut().insert(socket_id, cb_idx));
    Socket(socket_id)
}

/// Called by the host when a socket event is ready.
#[unsafe(no_mangle)]
pub extern "C" fn __on_socket_event(socket_id: u32, event_type: u32, data_ptr: u32, data_len: u32) {
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

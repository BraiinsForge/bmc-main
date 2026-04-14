// Copyright (C) 2026  Braiins Systems s.r.o.

// Guest-side endpoint (used by the relay daemon).
//
// `GuestEndpoint` owns the TCP listener and accepts connections in a loop.
// Multiple relay threads (framebuffer, LED, log tailers) each hold a
// `GuestSender` clone that remains valid across reconnections — sends
// silently drop when no host is connected.
//
// No raw TCP escapes this module.

use crate::types::{FrameHeader, GuestMessage, HostMessage, LedUpdate, LogSource};
use crate::wire;
use std::io::{self, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};

/// A message queued for the writer thread.
enum Queued {
    Msg(GuestMessage),
}

/// Channel depth: enough to absorb a few frames of latency without
/// blocking the relay's main loop. If the host can't keep up, sends
/// will block (backpressure), which is better than unbounded growth.
const CHANNEL_DEPTH: usize = 8;

/// Shared sender state — swapped atomically when a new host connects.
struct SenderState {
    tx: Mutex<Option<mpsc::SyncSender<Queued>>>,
}

impl std::fmt::Debug for SenderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let connected = self.tx.lock().is_ok_and(|guard| guard.is_some());
        f.debug_struct("SenderState")
            .field("connected", &connected)
            .finish()
    }
}

/// Cloneable sender handle — one per relay thread.
/// Remains valid across host reconnections. Sends silently drop
/// when no host is connected.
#[derive(Clone, Debug)]
pub struct GuestSender {
    inner: Arc<SenderState>,
}

impl GuestSender {
    /// Send a framebuffer frame. Pixel data is moved, not copied.
    /// Returns `Ok(())` even if no host is connected (frame is dropped).
    pub fn send_frame(&self, header: FrameHeader, pixels: Vec<u8>) {
        let msg = GuestMessage::Frame {
            header,
            data: pixels,
        };
        self.try_send(Queued::Msg(msg));
    }

    /// Send LED strip state.
    pub fn send_leds(&self, update: LedUpdate) {
        self.try_send(Queued::Msg(GuestMessage::Leds(update)));
    }

    /// Send a log line.
    pub fn send_log(&self, source: LogSource, line: String) {
        self.try_send(Queued::Msg(GuestMessage::Log { source, line }));
    }

    /// Send active effect index (0xFF = off).
    pub fn send_active_effect(&self, idx: u8) {
        self.try_send(Queued::Msg(GuestMessage::ActiveEffect(idx)));
    }

    /// Send volume state: app's configured volume + optional console override.
    pub fn send_volume(&self, app: u8, override_vol: Option<u8>) {
        self.try_send(Queued::Msg(GuestMessage::VolumeLevel { app, override_vol }));
    }

    /// Send capture availability.
    pub fn send_capture_status(&self, state: crate::types::FeatureState, reason: Option<String>) {
        self.try_send(Queued::Msg(GuestMessage::CaptureStatus { state, reason }));
    }

    /// Send controls availability.
    pub fn send_controls_status(&self, state: crate::types::FeatureState, reason: Option<String>) {
        self.try_send(Queued::Msg(GuestMessage::ControlsStatus { state, reason }));
    }

    /// Reply to a host Ping.
    pub fn send_pong(&self) {
        self.try_send(Queued::Msg(GuestMessage::Pong));
    }

    /// Send a notification to the host console.
    pub fn send_notify(&self, level: crate::types::NotifyLevel, message: String) {
        self.try_send(Queued::Msg(GuestMessage::Notify { level, message }));
    }

    /// Try to send a message. If no host is connected or the channel is
    /// broken, the message is silently dropped and the sender is cleared.
    fn try_send(&self, queued: Queued) {
        let mut guard = self
            .inner
            .tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(ref tx) = *guard
            && tx.try_send(queued).is_err()
        {
            // Channel full or disconnected — clear it so we don't
            // keep trying a dead channel.
            if tx.send(Queued::Msg(GuestMessage::Pong)).is_err() {
                // Truly disconnected (not just full), clear it
                *guard = None;
            }
            // If it was just full, we drop this message (backpressure)
        }
    }
}

/// Guest-side IPC endpoint. Owns the TCP listener and manages
/// connections across reconnects.
#[derive(Debug)]
pub struct GuestEndpoint {
    listener: TcpListener,
    sender: GuestSender,
}

impl GuestEndpoint {
    /// Bind to the given address and create the endpoint.
    /// Does NOT wait for a connection — call `accept_loop()` or
    /// `accept_next()` to start serving.
    pub fn bind(addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        eprintln!("ipc: listening on {addr}");

        let sender = GuestSender {
            inner: Arc::new(SenderState {
                tx: Mutex::new(None),
            }),
        };

        Ok(Self { listener, sender })
    }

    /// Get a cloneable sender for use by relay threads.
    /// The sender remains valid across reconnections.
    #[must_use]
    pub fn sender(&self) -> GuestSender {
        self.sender.clone()
    }

    /// Accept the next host connection. Blocks until a host connects.
    /// Spawns a writer thread and returns a reader for input events.
    /// When the host disconnects, the writer thread exits and the
    /// sender automatically starts dropping messages until the next
    /// `accept_next()` call.
    pub fn accept_next(&self) -> io::Result<GuestConnection> {
        let (stream, peer) = self.listener.accept()?;
        eprintln!("ipc: host connected from {peer}");

        let write_stream = stream.try_clone()?;
        let read_stream = stream;

        write_stream.set_nodelay(true)?;
        read_stream.set_nodelay(true)?;

        let (tx, rx) = mpsc::sync_channel::<Queued>(CHANNEL_DEPTH);

        // Install the new sender so all GuestSender clones start
        // delivering to this connection.
        {
            let mut guard = self
                .sender
                .inner
                .tx
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = Some(tx);
        }

        // Writer thread: drains the channel and serializes to TCP.
        let sender_inner = Arc::clone(&self.sender.inner);
        std::thread::Builder::new()
            .name("ipc-writer".into())
            .spawn(move || {
                let mut w = BufWriter::new(write_stream);
                for queued in rx {
                    match queued {
                        Queued::Msg(ref msg) => {
                            let data = if let GuestMessage::Frame { data, .. } = msg {
                                Some(data.as_slice())
                            } else {
                                None
                            };
                            if let Err(e) = wire::encode_guest(msg, data, &mut w) {
                                eprintln!("ipc writer error: {e}");
                                break;
                            }
                            if let Err(e) = w.flush() {
                                eprintln!("ipc flush error: {e}");
                                break;
                            }
                        }
                    }
                }
                // Clear the sender so other threads stop queueing
                if let Ok(mut guard) = sender_inner.tx.lock() {
                    *guard = None;
                }
                eprintln!("ipc: writer thread exiting");
            })?;

        Ok(GuestConnection {
            reader: BufReader::new(read_stream),
        })
    }
}

/// An active host connection. Returned by `accept_next()`.
/// Used by the relay to read input events from the host.
#[derive(Debug)]
pub struct GuestConnection {
    reader: BufReader<TcpStream>,
}

impl GuestConnection {
    /// Read the next message from the host.
    /// Returns `None` on EOF (host disconnected).
    pub fn recv(&mut self) -> io::Result<Option<HostMessage>> {
        wire::decode_host(&mut self.reader)
    }
}

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
    Wake,
    Msg(GuestMessage),
}

/// Channel depth for guest→host control messages (non-Frame/non-LED, those
/// flow through the latest-wins `pending` state). Sized to absorb log-tailer
/// replay bursts without dropping critical status messages.
const CHANNEL_DEPTH: usize = 64;

#[derive(Debug, Default)]
struct PendingState {
    frame: Option<(FrameHeader, Vec<u8>)>,
    led: Option<LedUpdate>,
}

/// Shared sender state — swapped atomically when a new host connects.
struct SenderState {
    tx: Mutex<Option<mpsc::SyncSender<Queued>>>,
    pending: Mutex<PendingState>,
}

impl std::fmt::Debug for SenderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let connected = self.tx.lock().is_ok_and(|guard| guard.is_some());
        let has_pending_frame = self.pending.lock().is_ok_and(|guard| guard.frame.is_some());
        let has_pending_led = self.pending.lock().is_ok_and(|guard| guard.led.is_some());
        f.debug_struct("SenderState")
            .field("connected", &connected)
            .field("has_pending_frame", &has_pending_frame)
            .field("has_pending_led", &has_pending_led)
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
        let mut guard = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.frame = Some((header, pixels));
        drop(guard);
        self.notify_pending();
    }

    /// Send LED strip state.
    pub fn send_leds(&self, update: LedUpdate) {
        let mut guard = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.led = Some(update);
        drop(guard);
        self.notify_pending();
    }

    /// Send a log line. Drops on overflow — logs are best-effort and must
    /// never stall the caller (log tailers are the highest-volume producer).
    pub fn send_log(&self, source: LogSource, line: String) {
        self.try_send_best_effort(Queued::Msg(GuestMessage::Log { source, line }));
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

    /// Reply to a host Ping. Called from the relay's input-reader thread,
    /// so it must not block — if the channel is full, the host will just
    /// time out on the next Ping and retry.
    pub fn send_pong(&self) {
        self.try_send_best_effort(Queued::Msg(GuestMessage::Pong));
    }

    /// Send a notification to the host console.
    pub fn send_notify(&self, level: crate::types::NotifyLevel, message: String) {
        self.try_send(Queued::Msg(GuestMessage::Notify { level, message }));
    }

    fn notify_pending(&self) {
        let tx = self
            .inner
            .tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(tx) = tx {
            match tx.try_send(Queued::Wake) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    if let Ok(mut guard) = self.inner.tx.lock() {
                        *guard = None;
                    }
                }
            }
        }
    }

    /// Blocking send for control messages that must be delivered in order
    /// (CaptureStatus, ControlsStatus, VolumeLevel, ActiveEffect, Notify).
    /// These are rare enough that backpressure from a 64-slot channel is
    /// not a concern in practice.
    ///
    /// If no host is connected or the channel is broken, the sender is
    /// cleared and the message is dropped.
    fn try_send(&self, queued: Queued) {
        let tx = self
            .inner
            .tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(tx) = tx {
            match tx.send(queued) {
                Ok(()) => {}
                Err(_) => {
                    if let Ok(mut guard) = self.inner.tx.lock() {
                        *guard = None;
                    }
                }
            }
        }
    }

    /// Non-blocking send for high-volume or latency-sensitive messages.
    /// Drops the message if the channel is full — used for logs (burst-heavy)
    /// and Pong (sent from the reader thread, where blocking would stall
    /// host-message processing).
    fn try_send_best_effort(&self, queued: Queued) {
        let tx = self
            .inner
            .tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(tx) = tx {
            match tx.try_send(queued) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    if let Ok(mut guard) = self.inner.tx.lock() {
                        *guard = None;
                    }
                }
            }
        }
    }
}

fn flush_pending(pending: &Mutex<PendingState>, w: &mut BufWriter<TcpStream>) -> io::Result<()> {
    let (frame, led) = {
        let mut guard = pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (guard.frame.take(), guard.led.take())
    };

    if let Some((header, data)) = frame {
        let msg = GuestMessage::Frame { header, data };
        if let GuestMessage::Frame { data, .. } = &msg {
            wire::encode_guest(&msg, Some(data.as_slice()), w)?;
        }
        w.flush()?;
    }

    if let Some(update) = led {
        let msg = GuestMessage::Leds(update);
        wire::encode_guest(&msg, None, w)?;
        w.flush()?;
    }

    Ok(())
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
                pending: Mutex::new(PendingState::default()),
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

        // Drop any pending Frame/LED left over from the previous host — the
        // new host hasn't seen those and the image/state may be from a stale
        // compositor session. The capture thread will re-populate pending.frame
        // on its next capture; callers (e.g. main.rs) explicitly re-push the
        // latest LED state for the new connection.
        {
            let mut guard = self
                .sender
                .inner
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = PendingState::default();
        }

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
        let sender_pending = Arc::clone(&self.sender.inner);
        std::thread::Builder::new()
            .name("ipc-writer".into())
            .spawn(move || {
                let mut w = BufWriter::new(write_stream);
                for queued in rx {
                    match queued {
                        Queued::Wake => {}
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
                    if let Err(e) = flush_pending(&sender_pending.pending, &mut w) {
                        eprintln!("ipc flush pending error: {e}");
                        break;
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

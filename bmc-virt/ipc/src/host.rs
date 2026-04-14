// Copyright (C) 2026  Braiins Systems s.r.o.

// Host-side endpoint (used by the console app).
//
// `HostEndpoint` connects to the relay, spawns a reader thread that
// delivers typed messages via an mpsc channel, and exposes a send
// method for input events. The console never touches raw TCP.

use crate::types::{GuestMessage, HostMessage, InputEvent};
use crate::wire;
use std::io::{self, BufReader, BufWriter, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

/// Host-side IPC endpoint.
pub struct HostEndpoint {
    rx: mpsc::Receiver<GuestMessage>,
    writer: BufWriter<TcpStream>,
    /// Set to `true` by the reader thread when the relay disconnects.
    disconnected: Arc<AtomicBool>,
}

impl std::fmt::Debug for HostEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostEndpoint")
            .field("disconnected", &self.disconnected.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// Channel depth for incoming guest messages.
/// Frames dominate bandwidth — keep a small buffer so the reader thread
/// doesn't allocate unbounded memory when the UI is slow.
const CHANNEL_DEPTH: usize = 4;

impl HostEndpoint {
    /// Connect to the relay at the given address.
    /// Spawns a reader thread that delivers `GuestMessage`s.
    pub fn connect(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;

        let read_stream = stream.try_clone()?;
        // Detect dead connections: the relay sends heartbeats every 500 ms,
        // so 2 s of silence means the guest is gone.
        read_stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
        let write_stream = stream;

        let (tx, rx) = mpsc::sync_channel::<GuestMessage>(CHANNEL_DEPTH);
        let disconnected = Arc::new(AtomicBool::new(false));
        let disc_flag = Arc::clone(&disconnected);

        // Reader thread: deserializes guest messages and sends them to the channel.
        std::thread::Builder::new()
            .name("ipc-reader".into())
            .spawn(move || {
                let mut r = BufReader::new(read_stream);
                loop {
                    match wire::decode_guest(&mut r) {
                        Ok(Some(msg)) => {
                            if tx.send(msg).is_err() {
                                break; // consumer dropped
                            }
                        }
                        Ok(None) => {
                            eprintln!("ipc: relay disconnected");
                            break;
                        }
                        Err(e) => {
                            if matches!(
                                e.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                            ) {
                                continue;
                            }
                            eprintln!("ipc reader error: {e}");
                            break;
                        }
                    }
                }
                disc_flag.store(true, Ordering::Release);
                eprintln!("ipc: reader thread exiting");
            })?;

        Ok(Self {
            rx,
            writer: BufWriter::new(write_stream),
            disconnected,
        })
    }

    /// Try to receive the next message from the relay (non-blocking).
    /// Returns `None` if no message is available.
    #[must_use]
    pub fn try_recv(&self) -> Option<GuestMessage> {
        self.rx.try_recv().ok()
    }

    /// Check whether the relay has disconnected.
    /// Uses an atomic flag set by the reader thread — does not
    /// consume any buffered messages.
    #[must_use]
    pub fn is_disconnected(&self) -> bool {
        self.disconnected.load(Ordering::Acquire)
    }

    /// Send an input event to the relay.
    pub fn send_input(&mut self, event: InputEvent) -> io::Result<()> {
        wire::encode_host(&HostMessage::Input(event), &mut self.writer)?;
        self.writer.flush()
    }

    /// Send a shell command to be executed on the guest.
    pub fn send_command(&mut self, cmd: &str) -> io::Result<()> {
        wire::encode_host(&HostMessage::RunCommand(cmd.to_owned()), &mut self.writer)?;
        self.writer.flush()
    }

    /// Send a GPIO reset button press/release event to the guest.
    pub fn send_gpio_button(&mut self, pressed: bool) -> io::Result<()> {
        wire::encode_host(&HostMessage::GpioButton { pressed }, &mut self.writer)?;
        self.writer.flush()
    }

    /// Send a liveness ping — the guest will reply with `GuestMessage::Pong`.
    pub fn send_ping(&mut self) -> io::Result<()> {
        wire::encode_host(&HostMessage::Ping, &mut self.writer)?;
        self.writer.flush()
    }
}

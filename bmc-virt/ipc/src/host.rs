// Copyright (C) 2026  Braiins Systems s.r.o.

// Host-side endpoint (used by the console app).
//
// `HostEndpoint` connects to the relay, spawns a reader thread that
// delivers typed messages via an mpsc channel, and exposes a send
// method for input events. The console never touches raw TCP.

use crate::types::{FrameHeader, GuestMessage, HostMessage, InputEvent, LedUpdate};
use crate::wire;
use std::io::{self, BufReader, BufWriter, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

type PendingFrame = Option<(FrameHeader, Vec<u8>)>;

/// Host-side IPC endpoint.
pub struct HostEndpoint {
    rx: mpsc::Receiver<GuestMessage>,
    latest_frame: Arc<Mutex<PendingFrame>>,
    latest_led: Arc<Mutex<Option<LedUpdate>>>,
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
/// Frames and LEDs bypass this channel (latest-wins mutex). What remains is
/// log lines + sparse control messages (Pong, CaptureStatus, etc.). Logs burst
/// hard when the guest's tailers replay backlog (e.g. bmc-openwrt restart), so
/// the channel needs to absorb a meaningful burst before backpressure kicks in.
const CHANNEL_DEPTH: usize = 64;

/// Route a decoded message to the right sink.
///
/// Frames and LEDs are sampled into their mutex slots (latest-wins). Log lines
/// are dropped on overflow — they're best-effort debug output, and dropping
/// them is preferable to stalling the reader thread (which would also stall
/// delivery of latency-sensitive control messages like CaptureStatus). Other
/// control messages still block on full channel: they're rare and delivery
/// matters, and the 64-deep channel makes genuine blocking unlikely.
///
/// Returns `false` when the consumer is gone (reader should exit).
fn route_incoming_message(
    msg: GuestMessage,
    tx: &mpsc::SyncSender<GuestMessage>,
    latest_frame: &Mutex<PendingFrame>,
    latest_led: &Mutex<Option<LedUpdate>>,
) -> bool {
    match msg {
        // Framebuffer state is also sampled: the console only needs
        // the most recent image, not a replay of stale frames.
        GuestMessage::Frame { header, data } => {
            let mut guard = latest_frame
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = Some((header, data));
            true
        }
        // LED state is a sampled view, not an ordered event log.
        // Keep only the newest pending value so a slow UI never
        // replays stale strip states in a burst.
        GuestMessage::Leds(update) => {
            let mut guard = latest_led
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = Some(update);
            true
        }
        log @ GuestMessage::Log { .. } => match tx.try_send(log) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        },
        other @ (GuestMessage::ActiveEffect(_)
        | GuestMessage::CaptureStatus { .. }
        | GuestMessage::Pong
        | GuestMessage::VolumeLevel { .. }
        | GuestMessage::ControlsStatus { .. }
        | GuestMessage::Notify { .. }) => tx.send(other).is_ok(),
    }
}

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
        let latest_frame = Arc::new(Mutex::new(None));
        let latest_led = Arc::new(Mutex::new(None));
        let disconnected = Arc::new(AtomicBool::new(false));
        let disc_flag = Arc::clone(&disconnected);
        let latest_frame_reader = Arc::clone(&latest_frame);
        let latest_led_reader = Arc::clone(&latest_led);

        // Reader thread: deserializes guest messages and sends them to the channel.
        std::thread::Builder::new()
            .name("ipc-reader".into())
            .spawn(move || {
                let mut r = BufReader::new(read_stream);
                loop {
                    match wire::decode_guest(&mut r) {
                        Ok(Some(msg)) => {
                            if !route_incoming_message(
                                msg,
                                &tx,
                                &latest_frame_reader,
                                &latest_led_reader,
                            ) {
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
            latest_frame,
            latest_led,
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

    /// Take the most recent pending framebuffer, if any.
    #[must_use]
    pub fn take_latest_frame(&self) -> Option<(FrameHeader, Vec<u8>)> {
        let mut guard = self
            .latest_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.take()
    }

    /// Take the most recent pending LED state, if any.
    #[must_use]
    pub fn take_latest_led(&self) -> Option<LedUpdate> {
        let mut guard = self
            .latest_led
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.take()
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

#[cfg(test)]
mod tests {
    use super::route_incoming_message;
    use crate::{
        Bpp, FrameHeader, GuestMessage, LED_COUNT, LedState, LedUpdate, LogSource, Stride,
    };
    use std::sync::{Mutex, mpsc};

    #[test]
    fn led_messages_overwrite_pending_state_instead_of_queueing() {
        let (tx, rx) = mpsc::sync_channel(4);
        let latest_frame = Mutex::new(None);
        let latest_led = Mutex::new(None);

        for seq in [1, 2] {
            assert!(route_incoming_message(
                GuestMessage::Leds(LedUpdate {
                    seq,
                    leds: [LedState::default(); LED_COUNT],
                }),
                &tx,
                &latest_frame,
                &latest_led,
            ));
        }

        assert!(rx.try_recv().is_err());
        let pending = latest_led
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .expect("BUG: latest LED state should be stored");
        assert_eq!(pending.seq, 2);
    }

    #[test]
    fn non_led_messages_still_use_fifo_channel() {
        let (tx, rx) = mpsc::sync_channel(4);
        let latest_frame = Mutex::new(None);
        let latest_led = Mutex::new(None);

        assert!(route_incoming_message(
            GuestMessage::Pong,
            &tx,
            &latest_frame,
            &latest_led,
        ));

        match rx
            .try_recv()
            .expect("BUG: queued message should be readable")
        {
            GuestMessage::Pong => {}
            GuestMessage::Frame { .. }
            | GuestMessage::Leds(_)
            | GuestMessage::Log { .. }
            | GuestMessage::ActiveEffect(_)
            | GuestMessage::CaptureStatus { .. }
            | GuestMessage::VolumeLevel { .. }
            | GuestMessage::ControlsStatus { .. }
            | GuestMessage::Notify { .. } => panic!("expected Pong"),
        }
        assert!(
            latest_led
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn frame_messages_overwrite_pending_state_instead_of_queueing() {
        let (tx, rx) = mpsc::sync_channel(4);
        let latest_frame = Mutex::new(None);
        let latest_led = Mutex::new(None);

        for (seq, fill) in [(1, 1_u8), (2, 2_u8)] {
            assert!(route_incoming_message(
                GuestMessage::Frame {
                    header: FrameHeader {
                        seq,
                        width: 480,
                        height: 1_280,
                        stride: Stride(1_920),
                        bpp: Bpp(32),
                        format: crate::PixelFormat::Rgba8888,
                        brightness: u8::MAX,
                    },
                    data: vec![fill; 16],
                },
                &tx,
                &latest_frame,
                &latest_led,
            ));
        }

        assert!(rx.try_recv().is_err());
        let (header, data) = latest_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .expect("BUG: latest frame should be stored");
        assert_eq!(header.seq, 2);
        assert_eq!(data, vec![2; 16]);
    }

    /// Log bursts must not stall the reader thread. When the channel is full,
    /// logs drop silently so that the next decoded message (possibly a
    /// latency-sensitive CaptureStatus) still gets in without waiting for the
    /// UI main thread to drain.
    #[test]
    fn log_messages_drop_on_overflow_instead_of_blocking() {
        let (tx, _rx) = mpsc::sync_channel::<GuestMessage>(2);
        let latest_frame = Mutex::new(None);
        let latest_led = Mutex::new(None);

        let make_log = || GuestMessage::Log {
            source: LogSource::BmcLog,
            line: "flood".to_owned(),
        };

        // Fill the channel (2 slots) then overflow with more logs. All calls
        // return `true` (keep running) because the sender side is still alive.
        for _ in 0..10 {
            assert!(route_incoming_message(
                make_log(),
                &tx,
                &latest_frame,
                &latest_led,
            ));
        }
    }

    /// Once the receiver is gone the reader should stop — both for logs and
    /// for blocking message kinds.
    #[test]
    fn sends_signal_exit_when_consumer_disconnects() {
        let latest_frame = Mutex::new(None);
        let latest_led = Mutex::new(None);
        let (tx, rx) = mpsc::sync_channel::<GuestMessage>(1);
        drop(rx);

        assert!(!route_incoming_message(
            GuestMessage::Log {
                source: LogSource::BmcLog,
                line: "after rx dropped".to_owned(),
            },
            &tx,
            &latest_frame,
            &latest_led,
        ));
        assert!(!route_incoming_message(
            GuestMessage::Pong,
            &tx,
            &latest_frame,
            &latest_led,
        ));
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

// Wire serialization / deserialization.
// All encoding details are confined to this module — neither guest.rs
// nor host.rs ever construct raw byte buffers.

use crate::protocol::{
    HEADER_SIZE, MAX_PAYLOAD, TAG_ACTIVE_EFFECT, TAG_CAPTURE_STATUS, TAG_CONTROLS_STATUS,
    TAG_FRAME, TAG_GPIO_BUTTON, TAG_INPUT, TAG_LEDS, TAG_LOG, TAG_NOTIFY, TAG_PING, TAG_PONG,
    TAG_RUN_COMMAND, TAG_VOLUME_LEVEL,
};
use crate::types::{
    Bpp, FeatureState, FrameHeader, GuestMessage, HostMessage, InputEvent, LED_COUNT, LedState,
    LedUpdate, LogSource, NotifyLevel, Stride,
};
use std::io::{self, Read, Write};

/// Extract a fixed-size array from a byte slice, returning an IO error on mismatch.
/// Used after length validation — the error path is unreachable in practice.
fn to_array<const N: usize>(slice: &[u8]) -> io::Result<[u8; N]> {
    slice
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "buffer too short"))
}

// ── Encoding (guest → host) ─────────────────────────────────────────────

/// Encode a `GuestMessage` onto a writer.
pub fn encode_guest(msg: &GuestMessage, data: Option<&[u8]>, w: &mut impl Write) -> io::Result<()> {
    match msg {
        GuestMessage::Frame { header, .. } => {
            // The caller passes the pixel data separately via `data` to
            // avoid an extra copy through the mpsc channel for large frames.
            let pixels = data.unwrap_or(&[]);
            // Payload: 8 (seq) + 4*4 (w,h,stride,bpp) + 1 (format) + 1 (brightness) + pixels
            let payload_len = 26 + pixels.len();
            write_header(w, TAG_FRAME, payload_len)?;
            w.write_all(&header.seq.to_le_bytes())?;
            w.write_all(&header.width.to_le_bytes())?;
            w.write_all(&header.height.to_le_bytes())?;
            w.write_all(&header.stride.0.to_le_bytes())?;
            w.write_all(&header.bpp.0.to_le_bytes())?;
            w.write_all(&[header.format.as_wire()])?;
            w.write_all(&[header.brightness])?;
            w.write_all(pixels)?;
        }
        GuestMessage::Leds(update) => {
            // Payload: 8 (seq) + LED_COUNT * 4
            let payload_len = 8 + LED_COUNT * 4;
            write_header(w, TAG_LEDS, payload_len)?;
            w.write_all(&update.seq.to_le_bytes())?;
            for led in &update.leds {
                w.write_all(&[led.brightness, led.r, led.g, led.b])?;
            }
        }
        GuestMessage::Log { source, line } => {
            // Payload: 1 (source) + line bytes
            let payload_len = 1 + line.len();
            write_header(w, TAG_LOG, payload_len)?;
            w.write_all(&[*source as u8])?;
            w.write_all(line.as_bytes())?;
        }
        GuestMessage::ActiveEffect(idx) => {
            write_header(w, TAG_ACTIVE_EFFECT, 1)?;
            w.write_all(&[*idx])?;
        }
        GuestMessage::CaptureStatus { state, reason }
        | GuestMessage::ControlsStatus { state, reason } => {
            let payload_len = 2 + reason.as_ref().map_or(0, String::len);
            let tag = if matches!(msg, GuestMessage::CaptureStatus { .. }) {
                TAG_CAPTURE_STATUS
            } else {
                TAG_CONTROLS_STATUS
            };
            write_header(w, tag, payload_len)?;
            w.write_all(&[*state as u8])?;
            let reason_bytes = reason.as_deref().unwrap_or("").as_bytes();
            w.write_all(&[u8::from(!reason_bytes.is_empty())])?;
            w.write_all(reason_bytes)?;
        }
        GuestMessage::Pong => {
            write_header(w, TAG_PONG, 0)?;
        }
        GuestMessage::VolumeLevel { app, override_vol } => {
            // Payload: 1 (app) + 1 (override: 0xFF = none, 0–100 = active)
            write_header(w, TAG_VOLUME_LEVEL, 2)?;
            w.write_all(&[*app, override_vol.unwrap_or(0xFF)])?;
        }
        GuestMessage::Notify { level, message } => {
            // Payload: 1 (level) + message bytes
            let payload_len = 1 + message.len();
            write_header(w, TAG_NOTIFY, payload_len)?;
            w.write_all(&[*level as u8])?;
            w.write_all(message.as_bytes())?;
        }
    }
    Ok(())
}

// ── Encoding (host → guest) ─────────────────────────────────────────────

/// Encode a `HostMessage` onto a writer.
pub fn encode_host(msg: &HostMessage, w: &mut impl Write) -> io::Result<()> {
    match msg {
        HostMessage::Input(event) => {
            // Payload: 1 (kind) + 2 (x) + 2 (y) + 1 (button) + 1 (data) = 7
            write_header(w, TAG_INPUT, 7)?;
            match event {
                InputEvent::TouchDown { x, y } => {
                    w.write_all(&[1])?;
                    w.write_all(&x.to_le_bytes())?;
                    w.write_all(&y.to_le_bytes())?;
                    w.write_all(&[0, 0])?;
                }
                InputEvent::TouchMove { x, y } => {
                    w.write_all(&[2])?;
                    w.write_all(&x.to_le_bytes())?;
                    w.write_all(&y.to_le_bytes())?;
                    w.write_all(&[0, 0])?;
                }
                InputEvent::TouchUp => {
                    w.write_all(&[3, 0, 0, 0, 0, 0, 0])?;
                }
                InputEvent::ButtonPress { button, data } => {
                    w.write_all(&[4, 0, 0, 0, 0, *button, *data])?;
                }
            }
        }
        HostMessage::RunCommand(cmd) => {
            write_header(w, TAG_RUN_COMMAND, cmd.len())?;
            w.write_all(cmd.as_bytes())?;
        }
        HostMessage::GpioButton { pressed } => {
            write_header(w, TAG_GPIO_BUTTON, 1)?;
            w.write_all(&[u8::from(*pressed)])?;
        }
        HostMessage::Ping => {
            write_header(w, TAG_PING, 0)?;
        }
    }
    Ok(())
}

// ── Decoding (guest → host) ─────────────────────────────────────────────

/// Decode the next `GuestMessage` from a reader.
/// Returns `None` on clean EOF.
pub fn decode_guest(r: &mut impl Read) -> io::Result<Option<GuestMessage>> {
    let Some((tag, payload)) = read_message(r)? else {
        return Ok(None);
    };
    if tag == TAG_FRAME {
        return decode_frame(&payload).map(Some);
    }
    decode_guest_payload(tag, &payload).map(Some)
}

fn decode_frame(payload: &[u8]) -> io::Result<GuestMessage> {
    if payload.len() < 26 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too short",
        ));
    }
    let seq = u64::from_le_bytes(to_array(&payload[0..8])?);
    let width = u32::from_le_bytes(to_array(&payload[8..12])?);
    let height = u32::from_le_bytes(to_array(&payload[12..16])?);
    let stride = Stride(u32::from_le_bytes(to_array(&payload[16..20])?));
    let bpp = Bpp(u32::from_le_bytes(to_array(&payload[20..24])?));
    let format = crate::types::PixelFormat::from_wire(payload[24]).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown pixel format byte: {}", payload[24]),
        )
    })?;
    let brightness = payload[25];
    let data = payload[26..].to_vec();
    Ok(GuestMessage::Frame {
        header: FrameHeader {
            seq,
            width,
            height,
            stride,
            bpp,
            format,
            brightness,
        },
        data,
    })
}

/// Parse a non-frame guest message from its wire tag and payload bytes.
fn decode_guest_payload(tag: u8, payload: &[u8]) -> io::Result<GuestMessage> {
    match tag {
        TAG_LEDS => {
            if payload.len() < 8 + LED_COUNT * 4 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "leds too short"));
            }
            let seq = u64::from_le_bytes(to_array(&payload[0..8])?);
            let mut leds = [LedState::default(); LED_COUNT];
            for (i, led) in leds.iter_mut().enumerate() {
                let off = 8 + i * 4;
                led.brightness = payload[off];
                led.r = payload[off + 1];
                led.g = payload[off + 2];
                led.b = payload[off + 3];
            }
            Ok(GuestMessage::Leds(LedUpdate { seq, leds }))
        }
        TAG_LOG => {
            if payload.is_empty() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "log too short"));
            }
            let source = LogSource::from_u8(payload[0])
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown log source"))?;
            let line = String::from_utf8_lossy(&payload[1..]).into_owned();
            Ok(GuestMessage::Log { source, line })
        }
        TAG_ACTIVE_EFFECT => {
            if payload.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "active_effect too short",
                ));
            }
            Ok(GuestMessage::ActiveEffect(payload[0]))
        }
        TAG_CAPTURE_STATUS | TAG_CONTROLS_STATUS => {
            if payload.len() < 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "feature status too short",
                ));
            }
            let state = FeatureState::from_u8(payload[0]).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "unknown feature state")
            })?;
            let reason = if payload[1] == 0 {
                None
            } else {
                Some(String::from_utf8_lossy(&payload[2..]).into_owned())
            };
            if tag == TAG_CAPTURE_STATUS {
                Ok(GuestMessage::CaptureStatus { state, reason })
            } else {
                Ok(GuestMessage::ControlsStatus { state, reason })
            }
        }
        TAG_PONG => Ok(GuestMessage::Pong),
        TAG_VOLUME_LEVEL => {
            if payload.len() < 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "volume_level too short",
                ));
            }
            let app = payload[0];
            let override_vol = if payload[1] == 0xFF {
                None
            } else {
                Some(payload[1])
            };
            Ok(GuestMessage::VolumeLevel { app, override_vol })
        }
        TAG_NOTIFY => {
            if payload.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "notify too short",
                ));
            }
            let level = NotifyLevel::from_u8(payload[0]).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "unknown notify level")
            })?;
            let message = String::from_utf8_lossy(&payload[1..]).into_owned();
            Ok(GuestMessage::Notify { level, message })
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown guest tag: {tag:#04X}"),
        )),
    }
}

/// Decode the next `HostMessage` from a reader.
/// Returns `None` on clean EOF.
pub fn decode_host(r: &mut impl Read) -> io::Result<Option<HostMessage>> {
    let Some((tag, payload)) = read_message(r)? else {
        return Ok(None);
    };

    match tag {
        TAG_INPUT => {
            if payload.len() < 7 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "input too short",
                ));
            }
            let kind = payload[0];
            let x = u16::from_le_bytes([payload[1], payload[2]]);
            let y = u16::from_le_bytes([payload[3], payload[4]]);
            let button = payload[5];
            let data = payload[6];

            let event = match kind {
                1 => InputEvent::TouchDown { x, y },
                2 => InputEvent::TouchMove { x, y },
                3 => InputEvent::TouchUp,
                4 => InputEvent::ButtonPress { button, data },
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unknown input kind: {kind}"),
                    ));
                }
            };
            Ok(Some(HostMessage::Input(event)))
        }
        TAG_RUN_COMMAND => {
            let cmd = String::from_utf8_lossy(&payload).into_owned();
            Ok(Some(HostMessage::RunCommand(cmd)))
        }
        TAG_GPIO_BUTTON => {
            if payload.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "gpio_button too short",
                ));
            }
            Ok(Some(HostMessage::GpioButton {
                pressed: payload[0] != 0,
            }))
        }
        TAG_PING => Ok(Some(HostMessage::Ping)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown host tag: {tag:#04X}"),
        )),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn write_header(w: &mut impl Write, tag: u8, payload_len: usize) -> io::Result<()> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "payload_len checked against MAX_PAYLOAD"
    )]
    let len = payload_len as u32;
    w.write_all(&[tag])?;
    w.write_all(&len.to_le_bytes())
}

/// Read a framed message: [tag: u8] [len: u32 LE] [payload].
/// Returns `None` on clean EOF (zero bytes read for the header).
fn read_message(r: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut hdr = [0_u8; HEADER_SIZE];
    match r.read_exact(&mut hdr) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let tag = hdr[0];
    let len = u32::from_le_bytes([hdr[1], hdr[2], hdr[3], hdr[4]]) as usize;

    if len > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("payload too large: {len} bytes"),
        ));
    }

    let mut payload = vec![0_u8; len];
    r.read_exact(&mut payload)?;
    Ok(Some((tag, payload)))
}

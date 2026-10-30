// Copyright (C) 2026  Jan Novák
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! The slice of MQTT 3.1.1 this widget needs to read a Venus OS broker:
//! CONNECT, SUBSCRIBE and PUBLISH out, CONNACK and PUBLISH in, PINGREQ to
//! keep the session up. QoS 0 only, so no packet ids need tracking beyond
//! the one SUBSCRIBE carries.

pub const TLS_PORT: u16 = 8883;
pub const PLAIN_PORT: u16 = 1883;

/// Seconds the broker waits without hearing from us before it drops the
/// session; we ping well inside it.
pub const KEEPALIVE_SECS: u16 = 60;

fn put_str(out: &mut Vec<u8>, s: &str) {
    let len = u16::try_from(s.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&s.as_bytes()[..usize::from(len)]);
}

fn put_remaining_len(out: &mut Vec<u8>, mut n: usize) {
    loop {
        #[expect(clippy::cast_possible_truncation, reason = "masked to 7 bits")]
        let mut byte = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            break;
        }
    }
}

fn packet(first: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(first);
    put_remaining_len(&mut out, body.len());
    out.extend_from_slice(body);
    out
}

/// CONNECT with a clean session and no credentials.
#[must_use]
pub fn connect(client_id: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(16 + client_id.len());
    put_str(&mut body, "MQTT");
    body.push(4); // protocol level 3.1.1
    body.push(0x02); // clean session
    body.extend_from_slice(&KEEPALIVE_SECS.to_be_bytes());
    put_str(&mut body, client_id);
    packet(0x10, &body)
}

/// SUBSCRIBE to `filters`, all at QoS 0.
#[must_use]
pub fn subscribe(packet_id: u16, filters: &[&str]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&packet_id.to_be_bytes());
    for f in filters {
        put_str(&mut body, f);
        body.push(0);
    }
    packet(0x82, &body)
}

/// PUBLISH at QoS 0, not retained.
#[must_use]
pub fn publish(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(topic.len() + payload.len() + 2);
    put_str(&mut body, topic);
    body.extend_from_slice(payload);
    packet(0x30, &body)
}

#[must_use]
pub fn pingreq() -> Vec<u8> {
    vec![0xC0, 0x00]
}

/// An incoming packet this widget cares about.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// CONNACK with its return code (0 = accepted).
    ConnAck(u8),
    Publish {
        topic: String,
        payload: Vec<u8>,
    },
    /// SUBACK, PINGRESP and anything else: proof of life, nothing to read.
    Other,
}

/// Reassembles packets from a byte stream that arrives in arbitrary chunks.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

/// Packets larger than this are dropped whole: Venus OS never sends one,
/// so it can only be a corrupt length and must not grow the buffer forever.
const MAX_PACKET: usize = 64 * 1024;

impl Decoder {
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Next complete packet, or `None` until more bytes arrive.
    pub fn next_packet(&mut self) -> Option<Incoming> {
        loop {
            let (len, header) = self.remaining_len()?;
            let total = header + len;
            if len > MAX_PACKET {
                self.buf.clear();
                return None;
            }
            if self.buf.len() < total {
                return None;
            }
            let first = self.buf[0];
            let body: Vec<u8> = self.buf[header..total].to_vec();
            self.buf.drain(..total);
            if let Some(p) = parse(first, &body) {
                return Some(p);
            }
        }
    }

    /// (remaining length, bytes taken by the fixed header) once both are in.
    fn remaining_len(&self) -> Option<(usize, usize)> {
        let mut len = 0usize;
        let mut mult = 1usize;
        for i in 1..5 {
            let byte = *self.buf.get(i)?;
            len += usize::from(byte & 0x7F) * mult;
            if byte & 0x80 == 0 {
                return Some((len, i + 1));
            }
            mult *= 128;
        }
        // Five length bytes is malformed; resynchronising is hopeless.
        Some((usize::MAX / 2, 5))
    }
}

/// `None` for a malformed packet, which is skipped.
fn parse(first: u8, body: &[u8]) -> Option<Incoming> {
    match first >> 4 {
        2 => Some(Incoming::ConnAck(*body.get(1)?)),
        3 => {
            let topic_len = usize::from(u16::from_be_bytes([*body.first()?, *body.get(1)?]));
            let topic = core::str::from_utf8(body.get(2..2 + topic_len)?).ok()?;
            let qos = (first >> 1) & 0x03;
            let start = 2 + topic_len + if qos > 0 { 2 } else { 0 };
            Some(Incoming::Publish {
                topic: topic.to_owned(),
                payload: body.get(start..)?.to_vec(),
            })
        }
        _ => Some(Incoming::Other),
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::wasm_test;

    use super::*;

    #[wasm_test]
    fn connect_is_a_clean_311_session_without_credentials() {
        let p = connect("deck");
        assert_eq!(p[0], 0x10);
        assert_eq!(usize::from(p[1]), p.len() - 2);
        assert_eq!(&p[2..8], b"\x00\x04MQTT");
        assert_eq!(p[8], 4, "protocol level 3.1.1");
        assert_eq!(p[9], 0x02, "clean session, no user/password flags");
        assert_eq!(&p[10..12], &KEEPALIVE_SECS.to_be_bytes());
        assert_eq!(&p[12..], b"\x00\x04deck");
    }

    #[wasm_test]
    fn subscribe_lists_every_filter_at_qos0() {
        let p = subscribe(7, &["a/#", "b"]);
        assert_eq!(p[0], 0x82);
        assert_eq!(&p[2..], b"\x00\x07\x00\x03a/#\x00\x00\x01b\x00");
    }

    #[wasm_test]
    fn publishes_are_reassembled_across_chunks_and_long_lengths() {
        let long_payload = vec![b'x'; 300]; // two-byte remaining length
        let mut wire = publish("N/abc/system/0/Serial", br#"{"value": "abc"}"#);
        wire.extend(publish("N/abc/x", &long_payload));
        wire.extend([0x20, 0x02, 0x00, 0x05]); // CONNACK, refused (5)
        let mut d = Decoder::default();
        let mut got = Vec::new();
        for chunk in wire.chunks(7) {
            d.push(chunk);
            while let Some(p) = d.next_packet() {
                got.push(p);
            }
        }
        assert_eq!(got.len(), 3);
        assert_eq!(
            got[0],
            Incoming::Publish {
                topic: "N/abc/system/0/Serial".into(),
                payload: br#"{"value": "abc"}"#.to_vec()
            }
        );
        assert!(matches!(&got[1], Incoming::Publish { payload, .. } if payload.len() == 300));
        assert_eq!(got[2], Incoming::ConnAck(5));
    }

    #[wasm_test]
    fn a_qos1_publish_skips_its_packet_id() {
        let mut d = Decoder::default();
        d.push(&[0x32, 0x07, 0x00, 0x01, b't', 0x00, 0x09, b'h', b'i']);
        assert_eq!(
            d.next_packet(),
            Some(Incoming::Publish {
                topic: "t".into(),
                payload: b"hi".to_vec()
            })
        );
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

//! The Deck's own network environment, delivered to wasm widgets
//! so a scene can name the Wi-Fi to join and the Deck's reachable address.

/// The connected network's name and the Deck's own address;
/// either is empty when unknown (no Wi-Fi, or before an address is assigned).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkInfo {
    pub ssid: String,
    pub ip: String,
}

/// Encode as `ssid_len:u16 | ssid | ip_len:u16 | ip`.
/// The SDK's `host_network_info` getter parses this shape into a `NetworkInfo`.
#[must_use]
pub fn encode(info: &NetworkInfo) -> Vec<u8> {
    let mut out = Vec::new();
    push_str(&mut out, &info.ssid);
    push_str(&mut out, &info.ip);
    out
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    let len = u16::try_from(s.len()).expect("BUG: SSID/IP length must fit in u16");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

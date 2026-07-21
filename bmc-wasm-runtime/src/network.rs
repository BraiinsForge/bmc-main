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

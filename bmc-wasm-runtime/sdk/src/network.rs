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

//! The Deck's own network environment (Wi-Fi SSID + IP).
//! A scene uses it to show a user how to reach the Deck — e.g. a QR-code link.

/// The connected network name and the Deck's own address.
/// Either is empty when the host doesn't know it (no Wi-Fi, or no address yet).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkInfo {
    pub ssid: String,
    pub ip: String,
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_network_info(out_ptr: *mut u8, out_cap: u32) -> u32;
}

/// Fetch the Deck's current network environment from the host.
/// Off-wasm (native stories/tests) returns an empty [`NetworkInfo`].
#[must_use]
pub fn info() -> NetworkInfo {
    #[cfg(target_arch = "wasm32")]
    {
        // SAFETY: the probe (out_cap 0) writes nothing and returns the length;
        // the real call writes at most `needed` bytes into a buffer of that size.
        let needed = unsafe { host_network_info(core::ptr::null_mut(), 0) };
        let mut buf = vec![0_u8; usize::try_from(needed).expect("BUG: size fits usize")];
        if needed > 0 {
            let written = unsafe { host_network_info(buf.as_mut_ptr(), needed) };
            assert!(
                written == needed,
                "BUG: host_network_info length changed between probe and read",
            );
        }
        parse(&buf)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        NetworkInfo::default()
    }
}

/// Decode `ssid_len:u16 | ssid | ip_len:u16 | ip`.
#[cfg(target_arch = "wasm32")]
fn parse(bytes: &[u8]) -> NetworkInfo {
    let mut cursor = 0;
    let ssid = read_str(bytes, &mut cursor);
    let ip = read_str(bytes, &mut cursor);
    NetworkInfo { ssid, ip }
}

#[cfg(target_arch = "wasm32")]
fn read_str(bytes: &[u8], cursor: &mut usize) -> String {
    if *cursor + 2 > bytes.len() {
        return String::new();
    }
    let len = usize::from(u16::from_le_bytes([bytes[*cursor], bytes[*cursor + 1]]));
    *cursor += 2;
    let end = (*cursor + len).min(bytes.len());
    let s = String::from_utf8_lossy(&bytes[*cursor..end]).into_owned();
    *cursor = end;
    s
}

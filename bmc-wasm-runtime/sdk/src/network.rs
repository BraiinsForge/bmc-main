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

/// Fetch the Deck's current network environment from the host.
/// Off-wasm (native stories/tests) returns an empty [`NetworkInfo`].
#[must_use]
pub fn info() -> NetworkInfo {
    parse(&Platform::info())
}

/// The answer behind [`info`], still encoded as on the wire.
trait NetworkInfoProvider {
    /// `ssid_len:u16 | ssid | ip_len:u16 | ip`.
    fn info() -> Vec<u8>;
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_network_info(out_ptr: *mut u8, out_cap: u32) -> u32;
    }

    pub(super) struct Host;

    impl super::NetworkInfoProvider for Host {
        fn info() -> Vec<u8> {
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
            buf
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    pub(super) struct Host;

    impl super::NetworkInfoProvider for Host {
        fn info() -> Vec<u8> {
            Vec::new()
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;

fn parse(bytes: &[u8]) -> NetworkInfo {
    let mut cursor = 0;
    let ssid = read_str(bytes, &mut cursor);
    let ip = read_str(bytes, &mut cursor);
    NetworkInfo { ssid, ip }
}

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

#[cfg(test)]
mod tests {
    use super::{NetworkInfo, parse};
    use crate::wasm_test;

    fn wire(ssid: &str, ip: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        for field in [ssid, ip] {
            let len = u16::try_from(field.len()).expect("BUG: a test field fits u16");
            bytes.extend_from_slice(&len.to_le_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes
    }

    #[wasm_test]
    fn both_fields_read_back() {
        assert_eq!(
            parse(&wire("home", "192.168.1.20")),
            NetworkInfo {
                ssid: "home".to_owned(),
                ip: "192.168.1.20".to_owned(),
            }
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod off_device {
        use crate::network::{NetworkInfo, info};

        #[test]
        fn info_off_target_is_the_empty_default() {
            assert_eq!(info(), NetworkInfo::default());
        }
    }

    #[wasm_test]
    fn no_bytes_read_as_the_empty_default() {
        assert_eq!(parse(&[]), NetworkInfo::default());
    }

    #[wasm_test]
    fn a_field_cut_short_keeps_what_arrived() {
        let mut bytes = wire("home", "192.168.1.20");
        bytes.truncate(bytes.len() - 3);
        assert_eq!(
            parse(&bytes),
            NetworkInfo {
                ssid: "home".to_owned(),
                ip: "192.168.1".to_owned(),
            }
        );
    }
}

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

// Re-exported so the family parsers keep naming
// `crate::discovery::JsonLookup` rather than the shared lib.
pub use mining::hashboards::JsonLookup;

use crate::device::DeviceFamily;
use crate::families::{bitaxe, bos};

/// The family an `_http._tcp` sighting's TXT identifies, if any.
#[must_use]
pub fn classify_http_found(json: &dyn JsonLookup) -> Option<DeviceFamily> {
    if bitaxe::is_axeos_txt(json) {
        Some(DeviceFamily::Bitaxe)
    } else if bos::is_bos_txt(json) {
        Some(DeviceFamily::Bos)
    } else {
        None
    }
}

/// Pull the user-facing name plus routing endpoint out of an mDNS `Found`
/// payload. Returns `None` when host or port are missing or unusable; the
/// caller's family is what classifies the device, not anything in here.
#[must_use]
pub fn extract_endpoint(json: &dyn JsonLookup) -> Option<(String, String, u16)> {
    let name = json.str("/name")?;
    let host = json.str("/host").filter(|h| !h.is_empty())?;
    let port = json
        .i64("/port")
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)?;
    Some((name, host, port))
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct MapJson {
        pub(crate) strings: BTreeMap<&'static str, &'static str>,
        pub(crate) ints: BTreeMap<&'static str, i64>,
        pub(crate) floats: BTreeMap<&'static str, f64>,
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).map(|s| (*s).to_owned())
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, path: &str) -> Option<f64> {
            self.floats.get(path).copied()
        }

        fn has(&self, path: &str) -> bool {
            let under = |key: &&str| key.starts_with(path);
            self.strings.keys().any(under)
                || self.ints.keys().any(under)
                || self.floats.keys().any(under)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::MapJson;
    use super::*;

    fn http_sighting() -> MapJson {
        let mut json = MapJson::default();
        json.strings.insert("/service_type", "_http._tcp.local.");
        json.strings.insert("/name", "miner-a._http._tcp.local.");
        json.strings.insert("/host", "10.0.0.5");
        json.ints.insert("/port", 80);
        json
    }

    #[test]
    fn extracts_name_host_and_port() {
        let (name, host, port) = extract_endpoint(&http_sighting()).expect("BUG: endpoint present");
        assert_eq!(name, "miner-a._http._tcp.local.");
        assert_eq!(host, "10.0.0.5");
        assert_eq!(port, 80);
    }

    #[test]
    fn rejects_event_missing_host() {
        let mut json = http_sighting();
        json.strings.remove("/host");
        assert_eq!(extract_endpoint(&json), None);
    }

    #[test]
    fn rejects_event_with_zero_port() {
        let mut json = http_sighting();
        json.ints.insert("/port", 0);
        assert_eq!(extract_endpoint(&json), None);
    }

    #[test]
    fn classifies_axeos_by_its_txt() {
        let mut json = http_sighting();
        json.strings.insert("/txt/board", "602");
        assert_eq!(classify_http_found(&json), Some(DeviceFamily::Bitaxe));
    }

    #[test]
    fn classifies_bos_by_its_txt() {
        let mut json = http_sighting();
        json.strings.insert("/txt/bos_version", "26.09");
        assert_eq!(classify_http_found(&json), Some(DeviceFamily::Bos));
    }

    #[test]
    fn a_plain_http_host_is_no_family() {
        let mut json = http_sighting();
        json.strings.insert("/txt/path", "/");
        assert_eq!(classify_http_found(&json), None);
    }
}

// Copyright (C) 2026  Braiins Systems s.r.o.

// The JSON lookup abstraction lives in the shared `mining` lib so this widget
// and `mining-info` parse hashboards through one implementation. Re-exported
// here so the family parsers keep referring to `crate::discovery::JsonLookup`.
pub use mining::hashboards::JsonLookup;

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
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::MapJson;
    use super::*;

    fn bos_shaped() -> MapJson {
        let mut json = MapJson::default();
        json.strings.insert("/service_type", "_http._tcp.local.");
        json.strings.insert("/name", "miner-a._http._tcp.local.");
        json.strings.insert("/host", "10.0.0.5");
        json.ints.insert("/port", 80);
        json
    }

    #[test]
    fn extracts_name_host_and_port() {
        let (name, host, port) = extract_endpoint(&bos_shaped()).expect("BUG: endpoint present");
        assert_eq!(name, "miner-a._http._tcp.local.");
        assert_eq!(host, "10.0.0.5");
        assert_eq!(port, 80);
    }

    #[test]
    fn rejects_event_missing_host() {
        let mut json = bos_shaped();
        json.strings.remove("/host");
        assert_eq!(extract_endpoint(&json), None);
    }

    #[test]
    fn rejects_event_with_zero_port() {
        let mut json = bos_shaped();
        json.ints.insert("/port", 0);
        assert_eq!(extract_endpoint(&json), None);
    }
}

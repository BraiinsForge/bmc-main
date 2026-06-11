// Copyright (C) 2026  Braiins Systems s.r.o.

//! Device display names: the hostname-like mDNS prefix, optionally mapped to
//! an operator-supplied friendly name.

/// The prefix of an mDNS instance name before the service-type suffix
/// (everything up to the first `._`): `"miner-a._http._tcp.local."` is
/// `"miner-a"`. Manual devices carry the bare host and pass through.
#[must_use]
pub fn display_name(identity_name: &str) -> &str {
    identity_name
        .split_once("._")
        .map_or(identity_name, |(prefix, _)| prefix)
}

/// RFC 6901 JSON-pointer token escaping: `~` to `~0`, `/` to `~1`. mDNS
/// instance names are arbitrary UTF-8 and may contain either.
#[must_use]
pub fn escape_pointer(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for c in key.chars() {
        match c {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            _ => out.push(c),
        }
    }
    out
}

/// A device's shown name: the operator mapping keyed by the display name,
/// then by the raw host (so IP-keyed entries match discovered devices, whose
/// host is the resolved IP), then the display name itself. `lookup` receives
/// unescaped keys.
#[must_use]
pub fn resolve(identity_name: &str, host: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let name = display_name(identity_name);
    lookup(name)
        .or_else(|| lookup(host))
        .unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_mdns_service_suffix_at_the_first_separator() {
        assert_eq!(display_name("miner-a._http._tcp.local."), "miner-a");
        assert_eq!(display_name("bmm-01._ubos._tcp.local."), "bmm-01");
        assert_eq!(
            display_name("Bitaxe Gamma 602 (A1B2)._http._tcp.local."),
            "Bitaxe Gamma 602 (A1B2)"
        );
    }

    #[test]
    fn manual_names_pass_through() {
        assert_eq!(display_name("10.0.0.5"), "10.0.0.5");
        assert_eq!(display_name("miner.local"), "miner.local");
    }

    #[test]
    fn escapes_json_pointer_tokens() {
        assert_eq!(escape_pointer("a/b~c"), "a~1b~0c");
        assert_eq!(escape_pointer("plain"), "plain");
        assert_eq!(escape_pointer("~~//"), "~0~0~1~1");
        assert_eq!(escape_pointer("čaj/řád"), "čaj~1řád");
    }

    #[test]
    fn resolves_by_display_name_then_host_then_falls_back() {
        let map = |key: &str| match key {
            "miner-a" => Some("Rack 3".to_owned()),
            "10.0.0.9" => Some("Bench".to_owned()),
            _ => None,
        };
        assert_eq!(
            resolve("miner-a._http._tcp.local.", "10.0.0.1", map),
            "Rack 3"
        );
        assert_eq!(resolve("other._http._tcp.local.", "10.0.0.9", map), "Bench");
        assert_eq!(resolve("other._http._tcp.local.", "10.0.0.1", map), "other");
        assert_eq!(
            resolve("miner-a._http._tcp.local.", "10.0.0.9", map),
            "Rack 3"
        );
    }
}

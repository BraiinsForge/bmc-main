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
}

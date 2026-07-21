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

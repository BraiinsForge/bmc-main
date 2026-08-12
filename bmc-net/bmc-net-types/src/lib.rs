// Copyright (C) 2024  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

//! Shared network and Wi-Fi data types for the `bmc-net` crate set.
//!
//! This crate is dependency-light and side-effect-free by design: it holds the
//! value types (`MacAddr`, network-protocol config, Wi-Fi status/scan items)
//! exchanged between the network manager (`bmc-net`), the drivers
//! (`bmc-net-drv`), and consumers such as `boser`. It performs no IO and no
//! logging so it can be linked anywhere without pulling in a runtime.

use std::fmt::{Display, Formatter};
use std::str::FromStr;

use thiserror::Error;

pub mod network;
pub mod wifi;

/// Format of the IP report packet. Available substitutions: "${MAC}", "${IP}", "${HOSTNAME}".
/// "${IP},${MAC}" is default since that is what stock IP reporter tool expects.
pub const DEFAULT_BROADCAST_FORMAT: &str = "${IP},${MAC}";

/// A 48-bit hardware (MAC) address.
///
/// `Display` renders the canonical lower-case colon-separated form
/// (`01:23:45:67:89:ab`); `FromStr` parses that same form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MacAddr {
    a: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    f: u8,
}

/// Error returned when a string cannot be parsed into a [`MacAddr`].
#[derive(Debug, Error, PartialEq, Eq)]
#[error("expected six colon-separated hex octets")]
pub struct MacAddrParseError;

impl MacAddr {
    /// Separator used between octets in the string representation.
    pub const DELIMITER: &str = ":";

    #[must_use]
    #[expect(clippy::many_single_char_names)]
    pub fn new(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) -> Self {
        Self { a, b, c, d, e, f }
    }
}

impl From<[u8; 6]> for MacAddr {
    fn from(octets: [u8; 6]) -> Self {
        Self {
            a: octets[0],
            b: octets[1],
            c: octets[2],
            d: octets[3],
            e: octets[4],
            f: octets[5],
        }
    }
}

impl FromStr for MacAddr {
    type Err = MacAddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut octets = [0_u8; 6];
        let mut parts = s.split(Self::DELIMITER);
        for octet in &mut octets {
            let part = parts.next().ok_or(MacAddrParseError)?;
            *octet = u8::from_str_radix(part, 16).map_err(|_| MacAddrParseError)?;
        }
        if parts.next().is_some() {
            return Err(MacAddrParseError);
        }
        Ok(Self::from(octets))
    }
}

impl Display for MacAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:02x}{}{:02x}{}{:02x}{}{:02x}{}{:02x}{}{:02x}",
            self.a,
            Self::DELIMITER,
            self.b,
            Self::DELIMITER,
            self.c,
            Self::DELIMITER,
            self.d,
            Self::DELIMITER,
            self.e,
            Self::DELIMITER,
            self.f
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_addr_string_format() {
        let mac_addr = MacAddr::new(0x01, 0x23, 0x45, 0x67, 0x89, 0xAB);

        assert_eq!(&mac_addr.to_string(), "01:23:45:67:89:ab");
    }

    #[test]
    fn mac_addr_parse_roundtrip() {
        let mac_addr = MacAddr::new(0x01, 0x23, 0x45, 0x67, 0x89, 0xAB);

        assert_eq!("01:23:45:67:89:ab".parse::<MacAddr>(), Ok(mac_addr));
        assert_eq!(mac_addr.to_string().parse::<MacAddr>(), Ok(mac_addr));
    }

    #[test]
    fn mac_addr_parse_rejects_malformed() {
        assert!("01:23:45:67:89".parse::<MacAddr>().is_err());
        assert!("01:23:45:67:89:ab:cd".parse::<MacAddr>().is_err());
        assert!("zz:23:45:67:89:ab".parse::<MacAddr>().is_err());
    }
}

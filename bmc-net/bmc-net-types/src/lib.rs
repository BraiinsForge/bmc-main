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

use std::fmt::{Display, Formatter, Result};

pub mod wifi;

/// Format of the IP report packet. Available substitutions: "${MAC}", "${IP}", "${HOSTNAME}".
/// "${IP},${MAC}" is default since that is what stock IP reporter tool expects.
pub const DEFAULT_BROADCAST_FORMAT: &str = "${IP},${MAC}";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MacAddr {
    a: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    f: u8,
}

impl MacAddr {
    pub const DELIMITER: &str = ":";

    #[must_use]
    #[expect(clippy::many_single_char_names)]
    pub fn new(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) -> Self {
        Self { a, b, c, d, e, f }
    }
}

impl Display for MacAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
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
}

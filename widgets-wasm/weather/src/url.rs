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

#[must_use]
pub fn weather_url(base: &str, location: &str) -> String {
    let mut out = String::from(base);
    for byte in location.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => push_percent_encoded(&mut out, other),
        }
    }
    out
}

fn push_percent_encoded(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0x0f) as usize] as char);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_become_percent_twenty() {
        let url = weather_url("https://example/api/weather/", "New York");
        assert_eq!(url, "https://example/api/weather/New%20York");
    }

    #[test]
    fn unreserved_ascii_passes_through() {
        assert_eq!(weather_url("b/", "Prague"), "b/Prague");
    }

    #[test]
    fn multibyte_umlaut_is_encoded_per_utf8_byte() {
        assert_eq!(weather_url("b/", "Zürich"), "b/Z%C3%BCrich");
    }

    #[test]
    fn multibyte_and_space_are_both_encoded() {
        assert_eq!(weather_url("b/", "São Paulo"), "b/S%C3%A3o%20Paulo");
    }
}

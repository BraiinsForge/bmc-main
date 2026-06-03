// Copyright (C) 2026  Braiins Systems s.r.o.

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

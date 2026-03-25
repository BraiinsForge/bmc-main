// Copyright (C) 2026  Braiins Systems s.r.o.

//! RFC 8259 string-body escaping for `json!` macro `#s(...)` interpolations.
//!
//! `JsonStr` wraps any `&str`-convertible value so that `ufmt`/[`crate::fmt!`]
//! writes it with `"`, `\`, and ASCII control characters properly escaped per
//! the JSON string grammar. Multi-byte UTF-8 sequences pass through unchanged.

use ufmt::{Formatter, uDisplay, uWrite};

/// Wrapper that escapes its inner value as a JSON string body when written
/// via `ufmt`.
///
/// The macro [`crate::json!`] wraps `#s(expr)` interpolations in this type so
/// the surrounding `"..."` template literal remains valid even when `expr`
/// contains characters that would otherwise terminate the string.
#[derive(Debug)]
pub struct JsonStr<T>(pub T);

impl<T: AsRef<str>> uDisplay for JsonStr<T> {
    fn fmt<W: uWrite + ?Sized>(&self, f: &mut Formatter<'_, W>) -> Result<(), W::Error> {
        for ch in self.0.as_ref().chars() {
            match ch {
                '"' => f.write_str("\\\"")?,
                '\\' => f.write_str("\\\\")?,
                '\n' => f.write_str("\\n")?,
                '\r' => f.write_str("\\r")?,
                '\t' => f.write_str("\\t")?,
                '\u{0008}' => f.write_str("\\b")?,
                '\u{000C}' => f.write_str("\\f")?,
                c if (c as u32) < 0x20 => {
                    let n = c as u32;
                    let hex = b"0123456789abcdef";
                    let buf = [
                        b'\\',
                        b'u',
                        b'0',
                        b'0',
                        hex[((n >> 4) & 0xf) as usize],
                        hex[(n & 0xf) as usize],
                    ];
                    // SAFETY: every byte was selected from the ASCII set above.
                    let escaped = unsafe { core::str::from_utf8_unchecked(&buf) };
                    f.write_str(escaped)?;
                }
                c => {
                    let mut utf8 = [0_u8; 4];
                    f.write_str(c.encode_utf8(&mut utf8))?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render<T: uDisplay>(v: T) -> String {
        let mut s = String::new();
        ufmt::uwrite!(&mut s, "{}", v).expect("BUG: String uWrite cannot fail");
        s
    }

    #[test]
    fn plain_passes_through() {
        assert_eq!(render(JsonStr("hello world")), "hello world");
    }

    #[test]
    fn double_quote_escaped() {
        assert_eq!(render(JsonStr(r#"a"b"#)), r#"a\"b"#);
    }

    #[test]
    fn backslash_escaped() {
        assert_eq!(render(JsonStr(r"a\b")), r"a\\b");
    }

    #[test]
    fn whitespace_chars() {
        assert_eq!(render(JsonStr("\n\r\t")), r"\n\r\t");
    }

    #[test]
    fn backspace_and_form_feed() {
        assert_eq!(render(JsonStr("\u{0008}\u{000C}")), r"\b\f");
    }

    #[test]
    fn other_control_chars_unicode_escape() {
        assert_eq!(render(JsonStr("\u{0001}\u{001F}")), r"\u0001\u001f");
    }

    #[test]
    fn utf8_passes_through() {
        assert_eq!(render(JsonStr("Příliš")), "Příliš");
        assert_eq!(render(JsonStr("🦀")), "🦀");
    }

    #[test]
    fn del_and_above_pass_through() {
        // RFC 8259 only requires escaping U+0000..U+001F plus `"` and `\`.
        // U+007F (DEL) and above pass through.
        assert_eq!(render(JsonStr("\u{007F}")), "\u{007F}");
    }

    #[test]
    fn injection_attempt_neutralised() {
        // Reviewer's concern: a value tries to break out of the string and
        // inject a sibling field.
        assert_eq!(
            render(JsonStr(r#"foo","admin":true,"x":"#)),
            r#"foo\",\"admin\":true,\"x\":"#,
        );
    }
}

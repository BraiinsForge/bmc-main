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

//! Reading received JSON by path, and writing outgoing JSON by hand.
//!
//! Writing avoids the `std` formatting macros,
//! which the widget build bans for their code size.

/// Read access to one received message by RFC 6901 JSON Pointer.
///
/// The widget reads through the host's `JsonDoc`, which answers lookups
/// but cannot list an object's keys,
/// so the core only ever asks for paths it can name.
pub trait JsonLookup {
    /// The string at `path`, `None` for anything else.
    fn str(&self, path: &str) -> Option<String>;
    /// [`Self::str`] for a string of at most `max` bytes, `None` for a longer one,
    /// which is never copied out.
    fn str_max(&self, path: &str, max: usize) -> Option<String>;
    /// The integer at `path`, `None` for anything else.
    fn i64(&self, path: &str) -> Option<i64>;
    /// The number at `path`, integer or not, `None` for anything else.
    fn f64(&self, path: &str) -> Option<f64>;
    /// The boolean at `path`, `None` for anything else.
    fn bool(&self, path: &str) -> Option<bool>;
    /// Element count of the array or object at `path`, `None` for anything else.
    fn len(&self, path: &str) -> Option<usize>;
    /// Whether anything is at `path`, `null` included.
    fn exists(&self, path: &str) -> bool;
}

/// A JSON Pointer, built one reference token at a time.
#[derive(Debug, Clone, Default)]
pub(crate) struct Pointer(String);

impl Pointer {
    /// The document root.
    pub(crate) fn root() -> Self {
        Self::default()
    }

    /// The member `token` of this object.
    pub(crate) fn key(&self, token: &str) -> Self {
        let mut path = self.0.clone();
        push_key(&mut path, token);
        Self(path)
    }
}

/// One pointer grown and cut back token by token over a single buffer,
/// to read many paths under one prefix without allocating each:
/// under fuel metering an allocation costs as much as dozens of lookups.
pub(crate) struct PointerBuf(String);

impl PointerBuf {
    pub(crate) fn new(root: Pointer) -> Self {
        Self(root.0)
    }

    /// Where the pointer ends now, for [`Self::truncate`] to come back to.
    pub(crate) fn mark(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn push(&mut self, token: &str) -> &str {
        push_key(&mut self.0, token);
        &self.0
    }

    /// Append `pointer`, already escaped, such as one a caller named.
    pub(crate) fn push_escaped(&mut self, pointer: &str) {
        self.0.push_str(pointer);
    }

    /// Append element `index` of the array the pointer names.
    pub(crate) fn push_index(&mut self, index: usize) -> &str {
        self.0.push('/');
        push_uint(&mut self.0, index as u64);
        &self.0
    }

    pub(crate) fn truncate(&mut self, mark: usize) {
        self.0.truncate(mark);
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Append the reference token `token`, with `~` and `/` escaped as RFC 6901 requires.
fn push_key(path: &mut String, token: &str) {
    path.push('/');
    for c in token.chars() {
        match c {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            other => path.push(other),
        }
    }
}

/// Builds one JSON object, member by member.
pub(crate) struct ObjectWriter {
    out: String,
    empty: bool,
}

impl ObjectWriter {
    pub(crate) fn new() -> Self {
        Self {
            out: String::from("{"),
            empty: true,
        }
    }

    fn member(&mut self, key: &str) {
        if !self.empty {
            self.out.push(',');
        }
        self.empty = false;
        push_string(&mut self.out, key);
        self.out.push(':');
    }

    pub(crate) fn string(mut self, key: &str, value: &str) -> Self {
        self.member(key);
        push_string(&mut self.out, value);
        self
    }

    pub(crate) fn uint(mut self, key: &str, value: u64) -> Self {
        self.member(key);
        push_uint(&mut self.out, value);
        self
    }

    pub(crate) fn int(mut self, key: &str, value: i64) -> Self {
        self.member(key);
        if value < 0 {
            self.out.push('-');
        }
        push_uint(&mut self.out, value.unsigned_abs());
        self
    }

    pub(crate) fn bool(mut self, key: &str, value: bool) -> Self {
        self.member(key);
        self.out.push_str(if value { "true" } else { "false" });
        self
    }

    pub(crate) fn strings<'a>(
        mut self,
        key: &str,
        values: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        self.member(key);
        self.out.push('[');
        for (i, value) in values.into_iter().enumerate() {
            if i > 0 {
                self.out.push(',');
            }
            push_string(&mut self.out, value);
        }
        self.out.push(']');
        self
    }

    pub(crate) fn object(mut self, key: &str, value: ObjectWriter) -> Self {
        self.member(key);
        self.out.push_str(&value.finish());
        self
    }

    pub(crate) fn finish(mut self) -> String {
        self.out.push('}');
        self.out
    }
}

/// `value` as a JSON string literal, escaped per RFC 8259 §7.
fn push_string(out: &mut String, value: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            control if u32::from(control) < 0x20 => {
                let code = u32::from(control);
                out.push_str("\\u00");
                for nibble in [code >> 4, code & 0xf] {
                    let digit = usize::try_from(nibble).expect("BUG: a nibble fits usize");
                    out.push(char::from(HEX[digit]));
                }
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

fn push_uint(out: &mut String, mut value: u64) {
    let mut digits = [0_u8; 20];
    let mut len = 0;
    loop {
        digits[len] = b'0' + u8::try_from(value % 10).expect("BUG: a decimal digit fits u8");
        len += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for &digit in digits[..len].iter().rev() {
        out.push(char::from(digit));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::JsonLookup;

    /// A received message, looked up the way the host's `JsonDoc` does.
    pub(crate) struct Message(serde_json::Value);

    impl Message {
        pub(crate) fn parse(text: &str) -> Self {
            Self(serde_json::from_str(text).expect("BUG: test messages are valid JSON"))
        }
    }

    impl JsonLookup for Message {
        fn str(&self, path: &str) -> Option<String> {
            self.0.pointer(path)?.as_str().map(str::to_owned)
        }

        fn str_max(&self, path: &str, max: usize) -> Option<String> {
            let text = self.0.pointer(path)?.as_str()?;
            (text.len() <= max).then(|| text.to_owned())
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.0.pointer(path)?.as_i64()
        }

        fn f64(&self, path: &str) -> Option<f64> {
            self.0.pointer(path)?.as_f64()
        }

        fn bool(&self, path: &str) -> Option<bool> {
            self.0.pointer(path)?.as_bool()
        }

        fn len(&self, path: &str) -> Option<usize> {
            match self.0.pointer(path)? {
                serde_json::Value::Array(items) => Some(items.len()),
                serde_json::Value::Object(members) => Some(members.len()),
                _ => None,
            }
        }

        fn exists(&self, path: &str) -> bool {
            self.0.pointer(path).is_some()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::Message;
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn a_pointer_escapes_tilde_and_slash() {
        let mut path = PointerBuf::new(Pointer::root().key("event").key("a/b~c"));
        path.push_index(12);

        assert_eq!(path.as_str(), "/event/a~1b~0c/12");
        assert_eq!(
            Message::parse(r#"{"event":{"a/b~c":[0,0,0,0,0,0,0,0,0,0,0,0,"found"]}}"#)
                .str(path.as_str())
                .as_deref(),
            Some("found"),
            "the escaped pointer must reach the member serde_json resolves"
        );
    }

    #[wasm_test]
    fn a_marked_buffer_cut_back_names_the_next_element_alone() {
        let mut path = PointerBuf::new(Pointer::root().key("result"));
        let array = path.mark();
        path.push_index(100);
        path.push("start");
        path.truncate(array);

        assert_eq!(path.push_index(7), "/result/7");
    }

    #[wasm_test]
    fn the_writer_produces_json_serde_reads_back() {
        let text = ObjectWriter::new()
            .uint("id", 18_446_744_073_709_551_615)
            .string("type", "call_service")
            .int("offset", -42)
            .bool("flag", false)
            .strings("entity_ids", ["light.a", "sensor.b"])
            .object(
                "target",
                ObjectWriter::new().string("entity_id", "button.gate"),
            )
            .finish();

        let read: serde_json::Value =
            serde_json::from_str(&text).expect("BUG: the writer emits valid JSON");
        assert_eq!(
            read,
            serde_json::json!({
                "id": 18_446_744_073_709_551_615_u64,
                "type": "call_service",
                "offset": -42,
                "flag": false,
                "entity_ids": ["light.a", "sensor.b"],
                "target": { "entity_id": "button.gate" },
            })
        );
    }

    #[wasm_test]
    fn a_string_survives_quotes_backslashes_and_control_characters() {
        let awkward = "say \"hi\"\\\n\t\u{1}\u{1f} ünïcode";
        let text = ObjectWriter::new().string("v", awkward).finish();

        let read: serde_json::Value =
            serde_json::from_str(&text).expect("BUG: the writer emits valid JSON");
        assert_eq!(read["v"], awkward);
    }

    #[wasm_test]
    fn an_empty_object_and_zero_are_written_plainly() {
        assert_eq!(ObjectWriter::new().finish(), "{}");
        assert_eq!(ObjectWriter::new().uint("n", 0).finish(), r#"{"n":0}"#);
    }
}

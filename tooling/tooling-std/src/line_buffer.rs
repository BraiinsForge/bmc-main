// Copyright (C) 2024  Braiins Systems s.r.o.

use bstr::ByteSlice;
use bytes::{BufMut, Bytes, BytesMut};
use std::mem;

#[derive(Default, Debug)]
pub struct LineBuffer {
    /// The underlying buffer.
    buf: BytesMut,
    /// There are no newlines in `buf` before this index.
    index_cache: usize,
}

impl LineBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append arbitrary data to the buffer.
    pub fn append_data(&mut self, data: impl Into<Bytes>) {
        self.buf.put(data.into());
    }

    /// Remove and return first full line in the buffer. Returns `None` if the
    /// buffer doesn't contain full line.
    pub fn next_line(&mut self) -> Option<Bytes> {
        // search the part of the buffer we haven't checked yet
        if let Some(index) = self.buf[self.index_cache..].find_byte(b'\n') {
            // already checked part + newly checked part + newline
            let index = self.index_cache + index + 1;

            // leave a full line in `self.buf`, return the rest
            let mut bytes = self.buf.split_off(index);

            // swap the buffers - we want to return the line and keep the rest
            mem::swap(&mut self.buf, &mut bytes);

            self.index_cache = 0;
            Some(bytes.freeze())
        } else {
            self.index_cache = self.buf.len();
            None
        }
    }

    #[must_use]
    pub fn remaining_data(&mut self) -> Bytes {
        mem::take(&mut self.buf).freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_buffer() {
        let mut buf = LineBuffer::new();

        buf.append_data("aaa");
        assert_eq!(buf.next_line(), None);
        buf.append_data("bbb\nccc");
        assert_eq!(buf.next_line().unwrap().as_bstr(), b"aaabbb\n".as_bstr());
        buf.append_data("\nd\ne\nfff");
        assert_eq!(buf.next_line().unwrap().as_bstr(), b"ccc\n".as_bstr());
        assert_eq!(buf.next_line().unwrap().as_bstr(), b"d\n".as_bstr());
        assert_eq!(buf.next_line().unwrap().as_bstr(), b"e\n".as_bstr());
        assert_eq!(buf.next_line(), None);
        assert_eq!(buf.remaining_data().as_bstr(), b"fff".as_bstr());
    }
}

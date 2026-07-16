// Copyright (C) 2025  Braiins Systems s.r.o.
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

use std::marker::PhantomData;

use bytes::BytesMut;
use serde::{Serialize, de::DeserializeOwned};
use tokio_util::codec::{Decoder, Encoder, LinesCodec, LinesCodecError};

/// Error that can occur during codec operations.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("encode error: {0}")]
    Encode(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("codec error: {0}")]
    Lines(#[from] LinesCodecError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Generic codec that combines newline framing with JSON encoding/decoding.
///
/// Type parameters:
/// - `Dec`: The type to decode from incoming messages
/// - `Enc`: The type to encode for outgoing messages
#[derive(Debug)]
pub struct JsonLinesCodec<Dec, Enc> {
    lines: LinesCodec,
    _phantom: PhantomData<(Dec, Enc)>,
}

impl<Dec, Enc> JsonLinesCodec<Dec, Enc> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: LinesCodec::new(),
            _phantom: PhantomData,
        }
    }
}

impl<Dec, Enc> Default for JsonLinesCodec<Dec, Enc> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Dec, Enc> Decoder for JsonLinesCodec<Dec, Enc>
where
    Dec: DeserializeOwned,
{
    type Item = Dec;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.lines.decode(src)? {
            Some(line) => {
                let msg =
                    serde_json::from_str(&line).map_err(|e| CodecError::Decode(e.to_string()))?;
                Ok(Some(msg))
            }
            None => Ok(None),
        }
    }
}

impl<Dec, Enc> Encoder<Enc> for JsonLinesCodec<Dec, Enc>
where
    Enc: Serialize,
{
    type Error = CodecError;

    fn encode(&mut self, item: Enc, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let json = serde_json::to_string(&item).map_err(|e| CodecError::Encode(e.to_string()))?;
        self.lines.encode(json, dst)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppMessage, WidgetMessage};

    #[test]
    fn encode_app_message() {
        let mut codec: JsonLinesCodec<WidgetMessage, AppMessage> = JsonLinesCodec::new();
        let mut buf = BytesMut::new();

        let msg = AppMessage::Shutdown;
        codec.encode(msg, &mut buf).expect("BUG: encode failed");

        let encoded = String::from_utf8_lossy(&buf);
        assert!(encoded.contains("shutdown"));
        assert!(encoded.ends_with('\n'));
    }

    #[test]
    fn decode_widget_message() {
        let mut codec: JsonLinesCodec<WidgetMessage, AppMessage> = JsonLinesCodec::new();
        let mut buf = BytesMut::from("{\"type\":\"ready\"}\n");

        let msg = codec.decode(&mut buf).expect("BUG: decode failed");

        assert!(matches!(msg, Some(WidgetMessage::Ready)));
    }

    #[test]
    fn decode_incomplete_message() {
        let mut codec: JsonLinesCodec<WidgetMessage, AppMessage> = JsonLinesCodec::new();
        let mut buf = BytesMut::from("{\"type\":\"ready\"");

        let msg = codec.decode(&mut buf).expect("BUG: decode failed");

        assert!(msg.is_none());
    }

    #[test]
    fn encode_widget_message() {
        let mut codec: JsonLinesCodec<AppMessage, WidgetMessage> = JsonLinesCodec::new();
        let mut buf = BytesMut::new();

        let msg = WidgetMessage::Ready;
        codec.encode(msg, &mut buf).expect("BUG: encode failed");

        let encoded = String::from_utf8_lossy(&buf);
        assert!(encoded.contains("ready"));
        assert!(encoded.ends_with('\n'));
    }

    #[test]
    fn decode_app_message() {
        let mut codec: JsonLinesCodec<AppMessage, WidgetMessage> = JsonLinesCodec::new();
        let mut buf = BytesMut::from("{\"type\":\"shutdown\"}\n");

        let msg = codec.decode(&mut buf).expect("BUG: decode failed");

        assert!(matches!(msg, Some(AppMessage::Shutdown)));
    }
}

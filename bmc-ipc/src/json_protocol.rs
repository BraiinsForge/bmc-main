// Copyright (C) 2025  Braiins Systems s.r.o.

use serde::{Serialize, de::DeserializeOwned};

use crate::{Protocol, ProtocolError};

#[derive(Debug, Default, Clone, Copy)]
pub struct JsonProtocol;

impl Protocol for JsonProtocol {
    fn encode<T: Serialize>(&self, message: &T) -> Result<Vec<u8>, ProtocolError> {
        serde_json::to_vec(message).map_err(|e| ProtocolError::Encode(e.to_string()))
    }

    fn decode<T: DeserializeOwned>(&self, bytes: &[u8]) -> Result<T, ProtocolError> {
        serde_json::from_slice(bytes).map_err(|e| ProtocolError::Decode(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestMessage {
        id: u32,
        name: String,
    }

    #[test]
    fn encode_produces_valid_json() {
        let protocol = JsonProtocol;
        let message = TestMessage {
            id: 42,
            name: "test".to_owned(),
        };

        let encoded = protocol
            .encode(&message)
            .expect("BUG: encoding should succeed");
        let parsed: serde_json::Value =
            serde_json::from_slice(&encoded).expect("BUG: output should be valid JSON");

        assert_eq!(parsed["id"], 42);
        assert_eq!(parsed["name"], "test");
    }

    #[test]
    fn decode_valid_json() {
        let protocol = JsonProtocol;
        let json_bytes = br#"{"id": 123, "name": "hello"}"#;

        let decoded: TestMessage = protocol
            .decode(json_bytes)
            .expect("BUG: decoding should succeed");

        assert_eq!(decoded.id, 123);
        assert_eq!(decoded.name, "hello");
    }

    #[test]
    fn decode_malformed_json() {
        let protocol = JsonProtocol;
        let invalid_bytes = b"not valid json";

        let result: Result<TestMessage, _> = protocol.decode(invalid_bytes);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProtocolError::Decode(_)));
    }

    #[test]
    fn decode_empty_input() {
        let protocol = JsonProtocol;
        let empty_bytes: &[u8] = b"";

        let result: Result<TestMessage, _> = protocol.decode(empty_bytes);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProtocolError::Decode(_)));
    }

    #[test]
    fn round_trip() {
        let protocol = JsonProtocol;
        let original = TestMessage {
            id: 999,
            name: "round trip test".to_owned(),
        };

        let encoded = protocol
            .encode(&original)
            .expect("BUG: encoding should succeed");
        let decoded: TestMessage = protocol
            .decode(&encoded)
            .expect("BUG: decoding should succeed");

        assert_eq!(original, decoded);
    }
}

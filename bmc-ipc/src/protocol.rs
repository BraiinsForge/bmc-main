// Copyright (C) 2025  Braiins Systems s.r.o.

use serde::{Serialize, de::DeserializeOwned};

use crate::ProtocolError;

/// Trait for encoding and decoding messages.
///
/// This trait defines how messages are serialized to bytes and deserialized from bytes.
pub trait Protocol {
    /// Encode a message to bytes.
    ///
    /// # Errors
    ///
    /// Returns `ProtocolError::Encode` if serialization fails.
    fn encode<T: Serialize>(&self, message: &T) -> Result<Vec<u8>, ProtocolError>;

    /// Decode bytes to a message.
    ///
    /// # Errors
    ///
    /// Returns `ProtocolError::Decode` if deserialization fails.
    fn decode<T: DeserializeOwned>(&self, bytes: &[u8]) -> Result<T, ProtocolError>;
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    /// A mock protocol that uses JSON for testing purposes.
    struct MockJsonProtocol;

    impl Protocol for MockJsonProtocol {
        fn encode<T: Serialize>(&self, message: &T) -> Result<Vec<u8>, ProtocolError> {
            serde_json::to_vec(message).map_err(|e| ProtocolError::Encode(e.to_string()))
        }

        fn decode<T: DeserializeOwned>(&self, bytes: &[u8]) -> Result<T, ProtocolError> {
            serde_json::from_slice(bytes).map_err(|e| ProtocolError::Decode(e.to_string()))
        }
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestMessage {
        id: u32,
        name: String,
    }

    #[test]
    fn encode_decode_round_trip() {
        let protocol = MockJsonProtocol;
        let original = TestMessage {
            id: 42,
            name: "test".to_owned(),
        };

        let encoded = protocol
            .encode(&original)
            .expect("BUG: encoding should succeed");
        let decoded: TestMessage = protocol
            .decode(&encoded)
            .expect("BUG: decoding should succeed");

        assert_eq!(original, decoded);
    }

    #[test]
    fn decode_invalid_input() {
        let protocol = MockJsonProtocol;
        let invalid_bytes = b"not valid json";

        let result: Result<TestMessage, _> = protocol.decode(invalid_bytes);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ProtocolError::Decode(_)));
    }

    #[test]
    fn decode_empty_input() {
        let protocol = MockJsonProtocol;
        let empty_bytes: &[u8] = b"";

        let result: Result<TestMessage, _> = protocol.decode(empty_bytes);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ProtocolError::Decode(_)));
    }
}

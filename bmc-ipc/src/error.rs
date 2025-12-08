// Copyright (C) 2025  Braiins Systems s.r.o.

use thiserror::Error;

/// Error type for protocol encoding/decoding operations.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// Failed to encode a message to bytes.
    #[error("failed to encode message: {0}")]
    Encode(String),

    /// Failed to decode bytes to a message.
    #[error("failed to decode message: {0}")]
    Decode(String),
}

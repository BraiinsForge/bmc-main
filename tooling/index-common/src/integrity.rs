// Copyright (C) 2023  Braiins Systems s.r.o.

use crate::sha256::Sha256Digest;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Integrity {
    pub checksum: Sha256Digest,
    pub size_bytes: usize,
    pub size_bytes_unpacked: Option<usize>,
    pub file_type: Option<String>,
    pub mime_type: Option<String>,
}

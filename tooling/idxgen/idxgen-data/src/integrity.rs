// Copyright (C) 2023  Braiins Systems s.r.o.

use infer::MatcherType;
use serde::{Deserialize, Serialize};
use tooling_std::sha256;
use tooling_std::sha256::Sha256Digest;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Integrity {
    pub checksum: Sha256Digest,
    pub size_bytes: usize,
    pub size_bytes_unpacked: Option<usize>,
    pub file_type: Option<String>,
    pub mime_type: Option<String>,
}

impl Integrity {
    pub fn from_file(data: impl AsRef<[u8]>) -> Self {
        let data = data.as_ref();

        let checksum = sha256::hash(data);
        let size_bytes = data.len();

        match infer::get(data) {
            None => Self {
                checksum,
                size_bytes,
                size_bytes_unpacked: None,
                file_type: None,
                mime_type: None,
            },
            Some(detected_type) => {
                if detected_type.matcher_type() == MatcherType::Archive {
                    // TODO(TOOL-844): try to decompress gz archives to find out if it's tar inside -> change file_type to "tar.gz"
                }

                Self {
                    checksum,
                    size_bytes,
                    size_bytes_unpacked: None,
                    file_type: Some(detected_type.extension().to_owned()),
                    mime_type: Some(detected_type.mime_type().to_owned()),
                }
            }
        }
    }

    pub fn for_dmg(data: impl AsRef<[u8]>) -> Self {
        let data = data.as_ref();

        let checksum = sha256::hash(data);
        let size_bytes = data.len();

        Self {
            checksum,
            size_bytes,
            size_bytes_unpacked: None,
            file_type: Some("dmg".to_owned()),
            mime_type: Some("application/x-apple-diskimage".to_owned()),
        }
    }
}

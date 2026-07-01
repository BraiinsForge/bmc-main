// Copyright (C) 2023  Braiins Systems s.r.o.

use crate::integrity::Integrity;
use crate::sha256::Sha256Digest;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum FileAsset {
    WithoutIntegrity(Url),
    WithIntegrity { url: Url, integrity: Integrity },
}

impl FileAsset {
    #[must_use]
    pub fn url(&self) -> &Url {
        match self {
            FileAsset::WithoutIntegrity(url) | FileAsset::WithIntegrity { url, .. } => url,
        }
    }

    #[must_use]
    pub fn checksum(&self) -> Option<Sha256Digest> {
        match self {
            FileAsset::WithoutIntegrity(_) => None,
            FileAsset::WithIntegrity { integrity, .. } => Some(integrity.checksum),
        }
    }

    #[must_use]
    pub fn size(&self) -> Option<usize> {
        match self {
            FileAsset::WithoutIntegrity(_) => None,
            FileAsset::WithIntegrity { integrity, .. } => Some(integrity.size_bytes),
        }
    }
}

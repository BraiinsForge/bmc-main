// Copyright (C) 2024  Braiins Systems s.r.o.

use crate::integrity::Integrity;
use crate::metadata::Input;
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use tooling_std::sha256::Sha256Digest;
use tracing::{error, warn};
use url::Url;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum SignedFileAsset<M> {
    WithoutIntegrity {
        url: Url,
        signature_url: Option<Url>,
        #[serde(flatten)]
        metadata: M,
    },
    WithIntegrity {
        url: Url,
        signature_url: Option<Url>,
        integrity: Integrity,
        #[serde(flatten)]
        metadata: M,
    },
}

impl<M: Clone> Input for SignedFileAsset<M> {
    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        output.insert(self.url().clone());
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        self.assign_integrity(lockfile);
    }
}

impl<M: Clone> SignedFileAsset<M> {
    pub fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        match (&self, lockfile.get(self.url())) {
            (
                SignedFileAsset::WithoutIntegrity {
                    url,
                    signature_url,
                    metadata,
                },
                Some(integrity),
            ) => {
                *self = SignedFileAsset::WithIntegrity {
                    url: url.clone(),
                    integrity: integrity.clone(),
                    signature_url: signature_url.clone(),
                    metadata: metadata.clone(),
                };
            }
            (SignedFileAsset::WithoutIntegrity { url, .. }, None) => {
                warn!("missing integrity for {url}");
            }
            (SignedFileAsset::WithIntegrity { url, integrity, .. }, Some(other_integrity)) => {
                if integrity != other_integrity {
                    error!("integrity mismatch for {url}");
                }
            }
            (SignedFileAsset::WithIntegrity { .. }, None) => {}
        }
    }

    #[must_use]
    pub fn url(&self) -> &Url {
        match self {
            SignedFileAsset::WithoutIntegrity { url, .. }
            | SignedFileAsset::WithIntegrity { url, .. } => url,
        }
    }

    #[must_use]
    pub fn signature_url(&self) -> Option<&Url> {
        match self {
            SignedFileAsset::WithoutIntegrity { signature_url, .. }
            | SignedFileAsset::WithIntegrity { signature_url, .. } => signature_url.as_ref(),
        }
    }

    #[must_use]
    pub fn checksum(&self) -> Option<Sha256Digest> {
        match self {
            SignedFileAsset::WithoutIntegrity { .. } => None,
            SignedFileAsset::WithIntegrity { integrity, .. } => Some(integrity.checksum),
        }
    }

    #[must_use]
    pub fn size(&self) -> Option<usize> {
        match self {
            SignedFileAsset::WithoutIntegrity { .. } => None,
            SignedFileAsset::WithIntegrity { integrity, .. } => Some(integrity.size_bytes),
        }
    }

    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        match self {
            SignedFileAsset::WithoutIntegrity { .. } => None,
            SignedFileAsset::WithIntegrity { integrity, .. } => integrity.file_type.as_deref(),
        }
    }

    #[must_use]
    pub fn mime_type(&self) -> Option<&str> {
        match self {
            SignedFileAsset::WithoutIntegrity { .. } => None,
            SignedFileAsset::WithIntegrity { integrity, .. } => integrity.mime_type.as_deref(),
        }
    }

    #[must_use]
    pub fn metadata(&self) -> &M {
        match self {
            SignedFileAsset::WithoutIntegrity { metadata, .. }
            | SignedFileAsset::WithIntegrity { metadata, .. } => metadata,
        }
    }

    #[must_use]
    pub fn metadata_mut(&mut self) -> &mut M {
        match self {
            SignedFileAsset::WithoutIntegrity { metadata, .. }
            | SignedFileAsset::WithIntegrity { metadata, .. } => metadata,
        }
    }
}

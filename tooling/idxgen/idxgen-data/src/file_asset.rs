// Copyright (C) 2023  Braiins Systems s.r.o.

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
pub enum FileAsset {
    WithoutIntegrity(Url),
    WithIntegrity { url: Url, integrity: Integrity },
}

impl Input for FileAsset {
    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        output.insert(self.url().clone());
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        self.assign_integrity(lockfile);
    }
}

impl FileAsset {
    pub fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        match (&self, lockfile.get(self.url())) {
            (FileAsset::WithoutIntegrity(url), Some(integrity)) => {
                *self = FileAsset::WithIntegrity {
                    url: url.clone(),
                    integrity: integrity.clone(),
                };
            }
            (FileAsset::WithoutIntegrity(url), None) => {
                warn!("missing integrity for {url}");
            }
            (FileAsset::WithIntegrity { url, integrity }, Some(other_integrity)) => {
                if integrity != other_integrity {
                    error!("integrity mismatch for {url}");
                }
            }
            (FileAsset::WithIntegrity { .. }, None) => {}
        }
    }

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

    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        match self {
            FileAsset::WithoutIntegrity(_) => None,
            FileAsset::WithIntegrity { integrity, .. } => integrity.file_type.as_deref(),
        }
    }

    #[must_use]
    pub fn mime_type(&self) -> Option<&str> {
        match self {
            FileAsset::WithoutIntegrity(_) => None,
            FileAsset::WithIntegrity { integrity, .. } => integrity.mime_type.as_deref(),
        }
    }
}

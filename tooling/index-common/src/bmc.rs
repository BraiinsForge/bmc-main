// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod v1;

use crate::metadata::ReleaseMetadata;
use serde::{Deserialize, Serialize};

/// *****************************************************************************
/// This is where we choose which version is the "latest" (and used in minerctl).
/// *****************************************************************************
pub use v1 as latest;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(tag = "metadata_version", content = "metadata")]
#[serde(rename_all = "lowercase")]
pub enum BmcReleaseMetadata {
    V1(v1::Metadata),
}

impl ReleaseMetadata for BmcReleaseMetadata {}

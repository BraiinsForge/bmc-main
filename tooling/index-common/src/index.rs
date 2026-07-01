// Copyright (C) 2023  Braiins Systems s.r.o.

use crate::bmc::BmcReleaseMetadata;
use crate::metadata::{MetadataVersion, ReleaseMetadata};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub enum Index {
    Bmc(IndexVariant<BmcReleaseMetadata>),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct IndexVariant<M: ReleaseMetadata> {
    pub status: IndexStatus,
    pub title: Option<String>,
    pub version: MetadataVersion,
    pub releases: Vec<Release<M>>,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
#[serde(deny_unknown_fields)]
pub enum IndexStatus {
    /// The most up to date index.
    Active,
    /// Index that contains all releases, but is missing some non-critical metadata.
    Outdated,
    /// Index that is too old, and there are new releases that aren't included in it.
    EndOfLife,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Release<M: ReleaseMetadata> {
    pub uuid: Uuid,
    #[serde(flatten)]
    pub metadata: Option<M>,
}

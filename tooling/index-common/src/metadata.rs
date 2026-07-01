// Copyright (C) 2023  Braiins Systems s.r.o.

use serde::{Deserialize, Serialize};
use strum::Display;

// TODO: MetadataVersion(u32)
#[derive(
    Serialize, Deserialize, Display, Default, Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq,
)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum MetadataVersion {
    #[default]
    V1,
    V2,
}

pub trait ReleaseMetadata: Clone {}

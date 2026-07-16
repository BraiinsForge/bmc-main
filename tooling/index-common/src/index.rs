// Copyright (C) 2023  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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

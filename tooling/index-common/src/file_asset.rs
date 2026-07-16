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

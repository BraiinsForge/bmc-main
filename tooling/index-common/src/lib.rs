// Copyright (C) 2024  Braiins Systems s.r.o.
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

pub mod bmc;
pub mod bos;
pub mod commit;
pub mod file_asset;
mod index;
pub mod integrity;
mod macro_rules;
pub mod metadata;
pub mod sha256;
pub mod url;

pub use index::{Index, IndexStatus, IndexVariant, Release};

use crate::metadata::ReleaseMetadata;
use crate::url::UrlExt;
use ::url::Url;
use fs_err as fs;
use itertools::Itertools;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tap::Pipe;
use thiserror::Error;

pub async fn download(
    index_url: &Url,
    version_name: &str,
    client: &Client,
    timeout: Duration,
) -> IndexResult<Index> {
    let url = index_url
        .join_path(format!("index.{version_name}.json"))
        .map_err(IndexError::InvalidUrl)?;

    let index = if url.scheme() == "file" {
        fs::read_to_string(url.path())
            .map_err(Arc::new)
            .map_err(IndexError::InvalidIndexPath)?
            .pipe_deref(serde_json::from_str::<Index>)
            .map_err(Arc::new)
            .map_err(IndexError::FailedToParse)?
    } else {
        client
            .get(url)
            .timeout(timeout)
            .send()
            .await?
            .error_for_status()?
            .json::<Index>()
            .await?
    };

    Ok(index)
}

// TODO: Serialize and Deserialize is used for caching the loaded index on disk, but more correct approach would be to
//       save the original (denormalized) index file that was originally downloaded from the server
/// This is a "normalized form" of an index. Index is normalized by converting all releases in the downloaded
/// (denormalized) index from various versions to a common structure that is used in the rest of the codebase.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NormalizedIndex<R: NormalizedRelease> {
    /// Status of this index. Check the enum variants for further explanation.
    pub status: IndexStatus,
    /// The list of releases.
    pub releases: Vec<R>,
    /// Number of releases accessible only in newer index versions.
    pub inaccessible_releases: usize,
}

impl<R: NormalizedRelease> NormalizedIndex<R> {
    #[must_use]
    pub fn normalize(index: IndexVariant<R::Denormalized>) -> Self {
        let (releases, inaccessible_releases) = index
            .releases
            .into_iter()
            .map(|release| match release.metadata {
                Some(metadata) => R::normalize(metadata),
                None => None,
            })
            .map(|opt| opt.ok_or(()))
            .partition_result::<Vec<_>, Vec<()>, _, _>()
            .pipe(|(releases, inaccessible)| (releases, inaccessible.len()));

        Self {
            status: index.status,
            releases,
            inaccessible_releases,
        }
    }
}

pub trait NormalizedRelease: Sized {
    type Denormalized: ReleaseMetadata;

    /// Convert a denormalized form into a normalized form. Return `None` when the release can't be normalized.
    fn normalize(release: Self::Denormalized) -> Option<Self>;
}

#[derive(Error, Debug, Clone)]
pub enum IndexError {
    #[error("invalid url")]
    InvalidUrl(#[source] ::url::ParseError),
    #[error("request failed")]
    RequestFailed(#[source] Arc<reqwest::Error>),
    #[error("invalid index type")]
    InvalidType,
    #[error("failed to parse index")]
    FailedToParse(#[source] Arc<serde_json::Error>),
    #[error("invalid index path")]
    InvalidIndexPath(#[source] Arc<io::Error>),
}

impl From<reqwest::Error> for IndexError {
    fn from(err: reqwest::Error) -> Self {
        if cfg!(debug_assertions) {
            Self::RequestFailed(Arc::new(err))
        } else {
            Self::RequestFailed(Arc::new(err.without_url()))
        }
    }
}

pub type IndexResult<T> = Result<T, IndexError>;

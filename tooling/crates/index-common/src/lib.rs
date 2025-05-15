// Copyright (C) 2024  Braiins Systems s.r.o.

use fs_err as fs;
use idxgen_data::metadata::ReleaseMetadata;
use idxgen_data::{IndexStatus, IndexVariant};
use itertools::Itertools;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tap::Pipe;
use thiserror::Error;
use tooling_std::url::UrlExt;
use url::Url;

pub async fn download(
    index_url: &Url,
    version_name: &str,
    client: &Client,
    timeout: Duration,
) -> IndexResult<idxgen_data::Index> {
    let url = index_url
        .join_path(format!("index.{version_name}.json"))
        .map_err(IndexError::InvalidUrl)?;

    let index = if url.scheme() == "file" {
        fs::read_to_string(url.path())
            .map_err(Arc::new)
            .map_err(IndexError::InvalidIndexPath)?
            .pipe_deref(serde_json::from_str::<idxgen_data::Index>)
            .map_err(Arc::new)
            .map_err(IndexError::FailedToParse)?
    } else {
        client
            .get(url)
            .timeout(timeout)
            .send()
            .await?
            .error_for_status()?
            .json::<idxgen_data::Index>()
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
    InvalidUrl(#[source] url::ParseError),
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

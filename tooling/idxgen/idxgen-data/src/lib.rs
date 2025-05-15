// Copyright (C) 2023  Braiins Systems s.r.o.

pub mod asset_state;
pub mod bma;
pub mod bmc;
pub mod bos;
mod file_asset;
#[cfg(feature = "html")]
mod html_render;
pub mod integrity;
pub mod metadata;
pub mod signed_file_asset;
pub mod toolbox;

use crate::bma::BmaReleaseMetadata;
use crate::bmc::BmcReleaseMetadata;
use crate::bos::BosReleaseMetadata;
use crate::metadata::{DowngradeOutput, MetadataVersion, ReleaseMetadata};
use crate::toolbox::ToolboxReleaseMetadata;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use uuid::Uuid;

pub const RELEASE_DATE_FORMAT: &str = "%B %-d, %Y";

// TODO: impl From<Index<ToolboxReleaseMetadata>> for IndexWrapper
#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub enum Index {
    Toolbox(IndexVariant<ToolboxReleaseMetadata>),
    Bos(IndexVariant<BosReleaseMetadata>),
    Bma(IndexVariant<BmaReleaseMetadata>),
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

////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////

impl Index {
    #[must_use]
    pub fn version(&self) -> MetadataVersion {
        match self {
            Index::Toolbox(index) => index.version,
            Index::Bos(index) => index.version,
            Index::Bma(index) => index.version,
            Index::Bmc(index) => index.version,
        }
    }

    /// Downgrade this index to an older version. Returns `None` when it's already on the oldest version.
    #[must_use]
    pub fn downgrade(&self) -> Option<Self> {
        Some(match self {
            Index::Toolbox(index) => Index::Toolbox(index.downgrade()?),
            Index::Bos(index) => Index::Bos(index.downgrade()?),
            Index::Bma(index) => Index::Bma(index.downgrade()?),
            Index::Bmc(index) => Index::Bmc(index.downgrade()?),
        })
    }

    #[must_use]
    pub fn summarize_releases(&self) -> Vec<String> {
        match self {
            Index::Toolbox(index) => index.summarize_releases(),
            Index::Bos(index) => index.summarize_releases(),
            Index::Bma(index) => index.summarize_releases(),
            Index::Bmc(index) => index.summarize_releases(),
        }
    }

    #[must_use]
    #[cfg(feature = "html")]
    pub fn render_html_document(&self) -> maud::Markup {
        match self {
            Index::Toolbox(index) => index.render_html_document(),
            Index::Bos(index) => index.render_html_document(),
            Index::Bma(index) => index.render_html_document(),
            Index::Bmc(index) => index.render_html_document(),
        }
    }
}

impl<M: ReleaseMetadata> IndexVariant<M> {
    /// Downgrade this index to an older version. Returns `None` when it's already on the oldest version.
    fn downgrade(&self) -> Option<Self> {
        // decrement version
        let version = match self.version {
            MetadataVersion::V1 => return None,
            MetadataVersion::V2 => MetadataVersion::V1,
        };

        // downgrade release metadata
        let releases = self
            .releases
            .iter()
            .map(|release| {
                let mut release = release.clone();

                release.metadata = release.metadata.and_then(|metadata| {
                    match metadata.metadata_version().cmp(&self.version) {
                        Ordering::Less => Some(metadata),
                        Ordering::Equal => match metadata.downgrade() {
                            DowngradeOutput::NonBreaking(metadata) => Some(metadata),
                            DowngradeOutput::Breaking | DowngradeOutput::Terminal => None,
                        },
                        Ordering::Greater => unreachable!(),
                    }
                });

                release
            })
            .collect_vec();

        // determine new status
        let status = match self.status {
            IndexStatus::Active | IndexStatus::Outdated => {
                if releases.iter().all(|r| r.metadata.is_some()) {
                    IndexStatus::Outdated
                } else {
                    IndexStatus::EndOfLife
                }
            }
            IndexStatus::EndOfLife => IndexStatus::EndOfLife,
        };

        Some(Self {
            status,
            title: self.title.clone(),
            version,
            releases,
        })
    }

    fn summarize_releases(&self) -> Vec<String> {
        self.releases
            .iter()
            .map(|release| match &release.metadata {
                None => format!("{}", release.uuid),
                Some(metadata) => format!(
                    "{} {:>4}   {}",
                    release.uuid,
                    metadata.metadata_version(),
                    metadata.release_title()
                ),
            })
            .collect_vec()
    }

    #[cfg(feature = "html")]
    fn render_html_document(&self) -> maud::Markup {
        use maud::{DOCTYPE, PreEscaped, html};

        const STYLESHEET: &str = include_str!("../assets/stylesheet.css");
        const SCRIPT: &str = include_str!("../assets/script.js");
        let icon_base_64: String = format!(
            "data:image/png;base64,{}",
            base64::encode(include_bytes!("../assets/icon.png"))
        );

        let releases = self
            .releases
            .iter()
            .filter_map(|release| release.metadata.as_ref().map(|m| (release.uuid, m)))
            .rev()
            .collect_vec();
        let (top_releases, other_releases) = releases.split_at(5.min(releases.len()));

        let title = self.title.as_deref().unwrap_or("Releases");

        html! {
            (DOCTYPE)
            html lang="en" {
                head {
                    meta charset="UTF-8";
                    link rel="shortcut icon" type="image/x-icon" href=(icon_base_64);
                    title { (title) }
                    meta name="viewport" content="width=device-width, initial-scale=1, user-scalable=no, shrink-to-fit=no";
                    style { (PreEscaped(STYLESHEET)) }
                }

                body {
                    header {
                        h1 { (title) }
                    }
                    @if releases.is_empty() {
                        p { "The list of releases is empty." }
                    }

                    section.anchors {
                        ul class="release-list" {
                            @for (uuid, metadata) in top_releases {
                                li {
                                    a href={ "#" (uuid) } { (metadata.release_title()) }
                                }
                            }
                            @if !other_releases.is_empty() {
                                li class="older-releases-wrapper" {
                                    input type="checkbox" id="older-releases-checkbox";
                                    label id="older-releases-btn" for="older-releases-checkbox" { "Older releases" }

                                    ul class="older-releases-list" {
                                        @for (uuid, metadata) in other_releases {
                                            li {
                                                a href={ "#" (uuid) } { (metadata.release_title()) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    div.releases {
                        @for (index, (uuid, metadata)) in releases.iter().enumerate() {
                            section.release #(uuid) {
                                header {
                                    h1.title {
                                        span {
                                            (metadata.release_title())
                                        }
                                        @if index == 0 {
                                            span.tag { "Latest" }
                                        }
                                    }
                                    div.date { (metadata.release_date()) }
                                }

                                main {
                                    (metadata.render_html())
                                }

                                footer {
                                    div.uuid title="UUID" {
                                        span.select.copy { (uuid) }
                                    }
                                }
                            }
                        }
                    }

                    button.scrollTop title="Scroll to top" {
                        span { "▲" }
                    }

                    script { (PreEscaped(SCRIPT)) }
                }
            }
        }
    }
}

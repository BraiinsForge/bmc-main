// Copyright (C) 2024  Braiins Systems s.r.o.

use crate::asset_state::AssetState;
use crate::file_asset::FileAsset;
use crate::signed_file_asset::SignedFileAsset;
use bytesize::ByteSize;
use maud::{Markup, html};
use tooling_std::display_option::DisplayNoneAs;
use tooling_std::sha256::Sha256Digest;
use url::Url;

pub struct HtmlAssetMetadata {
    pub name: String,
    pub url: Url,
    pub size: Option<String>,
    pub extension: Option<String>,
    pub checksum: Option<Sha256Digest>,
    pub signature_url: Option<Url>,
}
pub fn signed_assets_table(assets: &[HtmlAssetMetadata]) -> Markup {
    html! {
        div.table-wrapper {
            table.assets {
                thead {
                    tr {
                        th { "Asset" }
                        th { "Link" }
                        th { "Size" }
                        th { "Type" }
                        th { "SHA-256" }
                        th { "Signature" }
                    }
                }
                tbody {
                    @if assets.is_empty() {
                        tr.placeholder {
                            td colspan="100" { "There are no assets…" }
                        }
                    }
                    @for HtmlAssetMetadata {name, url, size, extension, checksum, signature_url} in assets {
                        tr {
                            td.name {
                                span.select.copy { (name) }
                            }
                            td.link { a href=(url) { "Download" } }
                            td.size { (size.display_none_as("-")) }
                            td.type { (extension.display_none_as("-")) }
                            td.checksum {
                                span.select.copy { (checksum.display_none_as("-")) }
                            }
                            td.signature {
                                @match signature_url {
                                    Some(url) => {
                                        a href=(url) { "Download" }
                                    },
                                    None => {
                                        "-"
                                    },
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn assets_table<'a>(assets: &[(&str, impl Into<Option<&'a FileAsset>> + Clone)]) -> Markup {
    let assets: Vec<_> = assets
        .iter()
        .filter_map(|(name, file)| file.clone().into().map(|file| (name, file)))
        .map(|(name, file)| {
            (
                name,
                file.url(),
                file.size()
                    .map(|s| ByteSize(s as u64).display().iec().to_string()),
                file.extension(),
                file.checksum(),
            )
        })
        .collect();

    html! {
        div.table-wrapper {
            table.assets {
                thead {
                    tr {
                        th { "Asset" }
                        th { "Link" }
                        th { "Size" }
                        th { "Type" }
                        th { "SHA-256" }
                    }
                }
                tbody {
                    @if assets.is_empty() {
                        tr.placeholder {
                            td colspan="100" { "No Assets…" }
                        }
                    }
                    @for (name, url, size, ext, checksum) in assets {
                        tr {
                            td.name {
                                span.select.copy { (name) }
                            }
                            td.link { a href=(url) { "Download" } }
                            td.size { (size.display_none_as("-")) }
                            td.type { (ext.display_none_as("-")) }
                            td.checksum {
                                span.select.copy { (checksum.display_none_as("-")) }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn signed_assets_html_metadata<M: Clone>(
    titled_assets: &[(&str, &AssetState<SignedFileAsset<M>>)],
) -> Vec<HtmlAssetMetadata> {
    titled_assets
        .iter()
        .filter_map(|(name, file)| match file {
            AssetState::None | AssetState::Yanked => None,
            AssetState::Available(file) => Some((name, file.clone())),
        })
        .map(|(name, file)| HtmlAssetMetadata {
            name: (*name).to_owned(),
            url: file.url().clone(),
            size: file
                .size()
                .map(|s| ByteSize(s as u64).display().iec().to_string()),
            extension: file.extension().map(ToOwned::to_owned),
            checksum: file.checksum(),
            signature_url: file.signature_url().cloned(),
        })
        .collect()
}

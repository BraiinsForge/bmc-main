// Copyright (C) 2023  Braiins Systems s.r.o.

use crate::{asset_state::AssetState, integrity::Integrity};
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use strum::Display;
use url::Url;

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

pub trait ReleaseMetadata: Clone + Default {
    /// Metadata version.
    fn metadata_version(&self) -> MetadataVersion;

    // Runs any pre-processing
    fn compile(&mut self);

    /// Collect all input URLs.
    fn collect_inputs(&self, output: &mut IndexSet<Url>);

    /// Add integrity info to all inputs.
    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>);

    /// Downgrade to an older version.
    fn downgrade(self) -> DowngradeOutput<Self>;

    /// A title of this release that can be displayed to the user.
    fn release_title(&self) -> String;

    /// A human readable (formatted) date of this release.
    fn release_date(&self) -> String;

    /// Convert this type to an HTML representation.
    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup;
}

pub trait ReleaseMetadataVersion {
    /// Collect all input URLs.
    fn collect_inputs(&self, output: &mut IndexSet<Url>);

    /// Add integrity info to all inputs.
    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>);

    /// An older version of this type.
    type Older;

    /// Downgrade to an older version.
    fn downgrade(self) -> DowngradeOutput<Self::Older>;

    /// Convert this type to an HTML representation.
    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup;
}

#[derive(Debug)]
pub enum DowngradeOutput<T> {
    /// Non-breaking change, downgrade is possible.
    NonBreaking(T),
    /// Breaking change, downgrade impossible.
    Breaking,
    /// There is no downgrade past this point (oldest version).
    Terminal,
}

impl<T> DowngradeOutput<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> DowngradeOutput<U> {
        match self {
            DowngradeOutput::NonBreaking(t) => DowngradeOutput::NonBreaking(f(t)),
            DowngradeOutput::Breaking => DowngradeOutput::Breaking,
            DowngradeOutput::Terminal => DowngradeOutput::Terminal,
        }
    }
}

pub trait Input {
    fn collect_inputs(&self, output: &mut IndexSet<Url>);
    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>);
}

impl<I: Input> Input for Option<I> {
    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        if let Some(this) = self {
            this.collect_inputs(output);
        }
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        if let Some(this) = self {
            this.assign_integrity(lockfile);
        }
    }
}

impl<I: Input + Clone> Input for AssetState<I> {
    fn collect_inputs(&self, output: &mut IndexSet<Url>) {
        if let AssetState::Available(this) = self {
            this.collect_inputs(output);
        }
    }

    fn assign_integrity(&mut self, lockfile: &IndexMap<Url, Integrity>) {
        if let AssetState::Available(this) = self {
            this.assign_integrity(lockfile);
        }
    }
}

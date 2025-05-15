// Copyright (C) 2023  Braiins Systems s.r.o.

use crate::integrity::Integrity;
use crate::metadata::{DowngradeOutput, ReleaseMetadataVersion};
use chrono::NaiveDate;
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use tooling_std::version::AppVersion;
use url::Url;

pub const VERSION_NAME: &str = "v1";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub toolbox_version: AppVersion,
    pub release_date: NaiveDate,
    pub description: String,
}

impl ReleaseMetadataVersion for Metadata {
    fn collect_inputs(&self, _output: &mut IndexSet<Url>) {
        // no inputs
    }

    fn assign_integrity(&mut self, _lockfile: &IndexMap<Url, Integrity>) {
        // no inputs
    }

    type Older = ();

    fn downgrade(self) -> DowngradeOutput<Self::Older> {
        DowngradeOutput::Terminal
    }

    #[cfg(feature = "html")]
    fn render_html(&self) -> maud::Markup {
        use maud::{PreEscaped, html};

        html! {
            div.markdown {
                (PreEscaped(markdown::to_html(&self.description)))
            }
        }
    }
}

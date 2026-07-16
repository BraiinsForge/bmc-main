// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
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

//! Scenario-driven [`FirmwareIndex`] pointing firmware releases at the
//! local blob server so downloads run offline.

use std::path::PathBuf;

use bmc_platform::BosPlatform;
use bmc_upgrade::firmware::{FirmwareDownloadError, FirmwareIndex, UpgradeMetadata};
use chrono::NaiveDate;
use reqwest::Client;

use crate::blob_server::BlobServer;
use crate::scenario::{self, FirmwareScenario, RunScenario};

const WRONG_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug)]
pub struct MockIndex {
    scenario_path: PathBuf,
    blob: BlobServer,
}

impl MockIndex {
    #[must_use]
    pub fn new(scenario_path: PathBuf, blob: BlobServer) -> Self {
        Self {
            scenario_path,
            blob,
        }
    }
}

fn parse_date(date: &str) -> NaiveDate {
    NaiveDate::parse_from_str(date, "%Y-%m-%d").expect("BUG: invalid release date literal")
}

#[async_trait::async_trait]
impl FirmwareIndex for MockIndex {
    async fn get_available_releases(
        &self,
        _client: &Client,
        _platform: BosPlatform,
        _version: String,
    ) -> Result<Option<Vec<UpgradeMetadata>>, FirmwareDownloadError> {
        let scenario = scenario::read(&self.scenario_path);

        match scenario.firmware {
            FirmwareScenario::UpToDate => return Ok(None),
            FirmwareScenario::CheckError => {
                return Err(FirmwareDownloadError::FetchUpgradeDetails);
            }
            FirmwareScenario::Available => {}
        }

        let (url, hash) = match scenario.run {
            RunScenario::DownloadFail => (self.blob.fail_url.clone(), self.blob.hash.clone()),
            RunScenario::HashMismatch => (self.blob.url.clone(), WRONG_HASH.to_owned()),
            RunScenario::Success | RunScenario::ApplyFail => {
                (self.blob.url.clone(), self.blob.hash.clone())
            }
        };

        let latest = UpgradeMetadata::new(
            hash,
            "2026-07-04-0-424f9a7f-26.07-plus".to_owned(),
            parse_date("2026-07-04"),
            "Introducing Braiins Deck Release 26.07".to_owned(),
            url,
            self.blob.size,
        );

        let previous_release1 = UpgradeMetadata::new(
            "4d3a1a9eadc995e06031e1f991f87845db359293432ecaabcb1ddb093227844f".to_owned(),
            "2026-05-21-0-e05df053-26.05-plus".to_owned(),
            parse_date("2026-05-21"),
            "Introducing Braiins Deck Release 26.05".to_owned(),
            "https://feeds.braiins-os.com/unused/previous1.tar".to_owned(),
            43_029_349,
        );

        let previous_release2 = UpgradeMetadata::new(
            "993c7bbf704947c4d52d0790b4ee8f284b0325a33b400d9089f143d5556f19d7".to_owned(),
            "2026-03-10-0-e0d0e70e-26.03-plus".to_owned(),
            parse_date("2026-03-10"),
            "Introducing Braiins Deck Release 26.03".to_owned(),
            "https://feeds.braiins-os.com/unused/previous2.tar".to_owned(),
            42_906_469,
        );

        Ok(Some(vec![latest, previous_release1, previous_release2]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_blob() -> crate::blob_server::BlobServer {
        crate::blob_server::BlobServer {
            url: "http://127.0.0.1:1/firmware.tar".to_owned(),
            fail_url: "http://127.0.0.1:1/firmware-fail.tar".to_owned(),
            hash: "a".repeat(64),
            size: 1000,
        }
    }

    fn write_scenario(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("upgrade-scenario.json");
        std::fs::write(&path, contents).expect("BUG: write scenario");
        path
    }

    async fn releases_for(
        scenario_json: &str,
    ) -> Result<Option<Vec<UpgradeMetadata>>, FirmwareDownloadError> {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = write_scenario(dir.path(), scenario_json);
        let index = MockIndex::new(path, test_blob());
        index
            .get_available_releases(&Client::new(), BosPlatform::Bmc1, "25".to_owned())
            .await
    }

    #[tokio::test]
    async fn available_points_latest_at_blob() {
        let releases = releases_for(r#"{"firmware": "available"}"#)
            .await
            .expect("BUG: check failed")
            .expect("BUG: expected releases");
        assert_eq!(releases[0].url, "http://127.0.0.1:1/firmware.tar");
        assert_eq!(releases[0].hash, "a".repeat(64));
        assert_eq!(releases[0].file_size, 1000);
        assert!(releases.len() > 1);
    }

    #[tokio::test]
    async fn up_to_date_returns_none() {
        let releases = releases_for(r#"{"firmware": "up-to-date"}"#)
            .await
            .expect("BUG: check failed");
        assert!(releases.is_none());
    }

    #[tokio::test]
    async fn check_error_returns_err() {
        assert!(
            releases_for(r#"{"firmware": "check-error"}"#)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn hash_mismatch_advertises_wrong_hash() {
        let releases = releases_for(r#"{"run": "hash-mismatch"}"#)
            .await
            .expect("BUG: check failed")
            .expect("BUG: expected releases");
        assert_ne!(releases[0].hash, "a".repeat(64));
        assert_eq!(releases[0].url, "http://127.0.0.1:1/firmware.tar");
    }

    #[tokio::test]
    async fn download_fail_advertises_fail_url() {
        let releases = releases_for(r#"{"run": "download-fail"}"#)
            .await
            .expect("BUG: check failed")
            .expect("BUG: expected releases");
        assert_eq!(releases[0].url, "http://127.0.0.1:1/firmware-fail.tar");
        assert_eq!(releases[0].hash, "a".repeat(64));
    }
}

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

use std::io::Write as _;
use std::path::Path;
use std::sync::Mutex;

use bmc_nix::feed::{PackageFeed, validate_feed};
use bmc_nix::store::{DownloadProgress, InitStoreError, SignatureVerification, init_store};
use bmc_nix::types::FactoryServerEntry;
use reqwest::header::{ACCEPT_ENCODING, RANGE};

const PREFIX_LEN: usize = 1024 * 1024;
const FEED_BASE_URL: &str = "https://downloads.braiinsforge.com/feeds/braiins-deck";
const TRUSTED_PUBLIC_KEY: &str =
    "downloads.braiinsforge.com-1:4XDOIc61MHtHeIOVrgNOfgzZHt4RCPfqWGDt5PwsLeU=";

#[derive(Default)]
struct RecordedProgress(Mutex<Vec<usize>>);

impl DownloadProgress for RecordedProgress {
    fn on_bytes_downloaded(&self, downloaded: usize, _: Option<usize>) {
        self.0
            .lock()
            .expect("BUG: progress lock poisoned")
            .push(downloaded);
    }

    fn on_extracting(&self) {}
}

fn seed_prefix(download_dir: &Path, bytes: &[u8], signature: &str) {
    let mut part = std::fs::File::create(download_dir.join("init-tarball.tar.gz.part"))
        .expect("create isolated partial");
    part.write_all(bytes).expect("seed CDN prefix");
    part.sync_all().expect("sync seeded prefix");
    let signature = serde_json::json!({ "signature": signature }).to_string();
    let mut metadata =
        std::fs::File::create(download_dir.join("init-tarball.tar.gz.metadata.json"))
            .expect("create isolated metadata");
    metadata
        .write_all(signature.as_bytes())
        .expect("seed matching signature");
    metadata.sync_all().expect("sync matching signature");
}

#[tokio::test]
#[ignore = "downloads a signed production tarball from downloads.braiinsforge.com"]
#[expect(
    clippy::too_many_lines,
    reason = "keep the two production CDN download attempts in one ignored scenario"
)]
async fn signed_resume_and_corruption_recovery_against_production_cdn() {
    let base_url =
        std::env::var("BMC_NIX_LIVE_FEED_BASE_URL").unwrap_or_else(|_| FEED_BASE_URL.to_owned());
    let key =
        std::env::var("BMC_NIX_LIVE_PUBLIC_KEY").unwrap_or_else(|_| TRUSTED_PUBLIC_KEY.to_owned());
    bmc_nix::signature::validate_public_key(&key).expect("valid production trust anchor");

    let client = reqwest::Client::new();
    let feed_url = bmc_nix::index::make_package_feed_url(&base_url);
    let feed: PackageFeed = client
        .get(&feed_url)
        .send()
        .await
        .expect("fetch production feed")
        .error_for_status()
        .expect("production feed status")
        .json()
        .await
        .expect("parse production feed");
    validate_feed(&feed_url, &feed).expect("valid production feed");
    let entry = feed
        .entries
        .iter()
        .filter(|entry| entry.signature.is_some())
        .max_by_key(|entry| &entry.bos_version)
        .expect("production feed has a signed init tarball");
    let url = reqwest::Url::parse(&entry.download_url).expect("valid tarball URL");
    assert_eq!(url.scheme(), "https", "live tarball must use HTTPS");
    assert_eq!(
        url.host_str(),
        Some("downloads.braiinsforge.com"),
        "live tarball must come from the production CDN"
    );

    let response = client
        .get(url)
        .header(ACCEPT_ENCODING, "identity")
        .header(RANGE, format!("bytes=0-{}", PREFIX_LEN - 1))
        .send()
        .await
        .expect("fetch production prefix");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::PARTIAL_CONTENT,
        "production CDN must serve byte ranges"
    );
    let prefix = response.bytes().await.expect("read production prefix");
    assert_eq!(prefix.len(), PREFIX_LEN, "complete production prefix");

    let tmp = tempfile::tempdir().expect("create isolated live-test directory");
    let download_dir = tmp.path().join("download");
    let stage_dir = tmp.path().join("stage");
    std::fs::create_dir(&download_dir).expect("create download directory");
    std::fs::create_dir(&stage_dir).expect("create staging directory");
    std::fs::create_dir(stage_dir.join("nix")).expect("block extraction after verification");
    let signature = entry.signature.as_deref().expect("signed entry");
    seed_prefix(&download_dir, &prefix, signature);

    let factory = FactoryServerEntry {
        id: "production-live-test".to_owned(),
        base_url,
        known_public_key: key.clone(),
        priority: 0,
        enabled: true,
    };
    let verification = SignatureVerification::Enabled {
        trusted_public_key: key.clone(),
    };
    let valid_progress = RecordedProgress::default();
    let result = init_store(
        &client,
        &factory,
        &entry.bos_version,
        &download_dir,
        &stage_dir,
        false,
        &verification,
        Some(&valid_progress),
    )
    .await;
    assert!(
        matches!(result, Err(InitStoreError::StoreAlreadyExists { .. })),
        "signed download should finish and verify before the deliberate extraction guard: {result:?}"
    );
    assert!(
        download_dir.join("init-tarball.tar.gz").is_file(),
        "verified final tarball should remain after the extraction guard"
    );
    let verified_size = std::fs::metadata(download_dir.join("init-tarball.tar.gz"))
        .expect("verified final size")
        .len();
    let verified_size = usize::try_from(verified_size).expect("test tarball fits in usize");
    assert!(
        !download_dir.join("init-tarball.tar.gz.part").exists(),
        "resumed partial should be promoted after verification"
    );
    let valid_events = valid_progress
        .0
        .lock()
        .expect("BUG: progress lock poisoned")
        .clone();
    assert_eq!(valid_events.first(), Some(&PREFIX_LEN));
    assert!(
        !valid_events.contains(&0),
        "a CDN range response should not restart the valid prefix"
    );

    std::fs::remove_file(download_dir.join("init-tarball.tar.gz"))
        .expect("remove first verified temporary download");
    let mut damaged = prefix.to_vec();
    damaged[0] ^= 1;
    seed_prefix(&download_dir, &damaged, signature);
    let damaged_progress = RecordedProgress::default();
    let result = init_store(
        &client,
        &factory,
        &entry.bos_version,
        &download_dir,
        &stage_dir,
        false,
        &verification,
        Some(&damaged_progress),
    )
    .await;
    assert!(
        matches!(result, Err(InitStoreError::StoreAlreadyExists { .. })),
        "damaged retained bytes should trigger one clean signed download: {result:?}"
    );
    let damaged_events = damaged_progress
        .0
        .lock()
        .expect("BUG: progress lock poisoned");
    assert_eq!(damaged_events.first(), Some(&PREFIX_LEN));
    assert!(
        damaged_events
            .windows(2)
            .any(|pair| pair[0] == verified_size && pair[1] == 0),
        "a complete but corrupt resumed candidate must reset progress before the clean retry: {damaged_events:?}"
    );
    assert!(
        download_dir.join("init-tarball.tar.gz").is_file(),
        "the clean retry should leave a verified final tarball"
    );
}

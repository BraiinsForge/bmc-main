// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_platform::BmcPlatform;
use bmc_upgrade::firmware::{FirmwareDownloadError, FirmwareIndex, UpgradeMetadata};
use chrono::NaiveDate;
use reqwest::Client;
#[derive(Debug)]
pub struct MockIndex;

#[async_trait::async_trait]
impl FirmwareIndex for MockIndex {
    async fn get_available_releases(
        &self,
        _client: &Client,
        _platform: BmcPlatform,
        _version: String,
    ) -> Result<Option<Vec<UpgradeMetadata>>, FirmwareDownloadError> {
        let release = UpgradeMetadata::new(
            "c6dfec40fa461274fba58d337df637e266b9db73fea6d660e0914167c298af3b".to_owned(),"2025-04-07-0-424f9a7f-25.03-plus".to_owned(), NaiveDate::parse_from_str("2025-04-07", "%Y-%m-%d")
            .expect("BUG: Failed to parse date"),
            "Introducing Braiins Clock Release 0.1.0".to_owned(),
            "https://feeds.braiins-os.com/cvitek-bm1-am2/firmware_2025-04-07-0-424f9a7f-25.03-plus_aarch64_armv8-a.tar".to_owned(),
            29736960
        );

        let previous_release1 = UpgradeMetadata::new("4d3a1a9eadc995e06031e1f991f87845db359293432ecaabcb1ddb093227844f".to_owned(),  "2025-02-21-0-e05df053-25.01-plus".to_owned(), NaiveDate::parse_from_str("2025-02-21", "%Y-%m-%d")
            .expect("BUG: Failed to parse date"),"Introducing Braiins Clock Release 0.1.0-beta".to_owned(), "https://feeds.braiins-os.com/stm32mp15-ii1-am2/firmware_2025-02-21-0-e05df053-25.01-plus_arm_cortex-a7_neon-vfpv4.tar".to_owned(), 43029349);

        let previous_release2 = UpgradeMetadata::new("993c7bbf704947c4d52d0790b4ee8f284b0325a33b400d9089f143d5556f19d7".to_owned(), "2024-12-10-0-e0d0e70e-24.12-plus".to_owned(),NaiveDate::parse_from_str("2024-12-10", "%Y-%m-%d")
        .expect("BUG: Failed to parse date"), "Introducing Braiins Clock Release 0.1.0-alfa".to_owned(), "https://feeds.braiins-os.com/stm32mp15-ii1-am2/firmware_2024-12-10-0-e0d0e70e-24.12-plus_arm_cortex-a7_neon-vfpv4.tar".to_owned(), 42906469);

        return Ok(Some(vec![release, previous_release1, previous_release2]));
    }
}

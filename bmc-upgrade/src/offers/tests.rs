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

use super::*;

use std::collections::BTreeMap;

fn packages(version: &str) -> PackageOffer {
    PackageOffer {
        index: MergedIndex {
            packages: Vec::new(),
            by_name: BTreeMap::new(),
        },
        preview: PackagesPreview {
            changes: Vec::new(),
            download_size_bytes: None,
            unpacked_size_bytes: None,
            bmc_version: Some(version.to_owned()),
            bmc_changelog: None,
        },
    }
}

async fn check(
    offers: &mut UpgradeOfferCache<&'static str>,
    firmware: Option<&'static str>,
    packages: Option<PackageOffer>,
) -> OfferCheck<&'static str> {
    offers.invalidate();
    let prepared = prepare(vec!["requested".to_owned()], firmware, async {
        Ok::<_, ()>(packages)
    })
    .await
    .expect("BUG: fixture probes succeed");
    offers.cache(prepared)
}

#[tokio::test]
async fn direct_preparation_does_not_replace_an_interactive_offer_or_its_install_intent() {
    let mut offers = UpgradeOfferCache::default();
    let id = check(&mut offers, None, Some(packages("interactive")))
        .await
        .upgrade_id
        .expect("BUG: interactive offer exists");
    let automatic = prepare(Vec::new(), None::<&str>, async {
        Ok::<_, ()>(Some(packages("automatic")))
    })
    .await
    .expect("BUG: automatic preparation succeeds");
    assert!(
        matches!(automatic.upgrade, Some(UpgradeOffer::Packages { packages, install }) if packages.preview.bmc_version.as_deref() == Some("automatic") && install.is_empty())
    );
    assert!(
        matches!(offers.claim(id), Some(UpgradeOffer::Packages { packages, install }) if packages.preview.bmc_version.as_deref() == Some("interactive") && install == ["requested"])
    );
}

#[tokio::test]
async fn firmware_wins_but_both_previews_are_returned() {
    let mut offers = UpgradeOfferCache::default();
    let result = check(&mut offers, Some("firmware"), Some(packages("packages"))).await;
    assert_eq!(result.firmware, Some("firmware"));
    assert_eq!(
        result
            .packages
            .as_ref()
            .and_then(|preview| preview.bmc_version.as_deref()),
        Some("packages")
    );
    assert_eq!(result.disruption, Disruption::Reboot);
    let offer = offers.claim(result.upgrade_id.expect("BUG: firmware offer"));
    assert!(
        matches!(offer, Some(UpgradeOffer::Firmware { firmware: "firmware", package_preview: Some(packages), install }) if packages.bmc_version.as_deref() == Some("packages") && install == ["requested"])
    );
}

#[tokio::test]
async fn packages_only_retains_checked_payload_and_install() {
    let mut offers = UpgradeOfferCache::default();
    let result = check(&mut offers, None, Some(packages("checked-index"))).await;
    assert_eq!(result.disruption, Disruption::AppRestart);
    let offer = offers.claim(result.upgrade_id.expect("BUG: package offer"));
    assert!(
        matches!(offer, Some(UpgradeOffer::Packages { packages, install }) if packages.preview.bmc_version.as_deref() == Some("checked-index") && install == ["requested"])
    );
}

#[tokio::test]
async fn no_changes_produces_no_id() {
    let result = check(&mut UpgradeOfferCache::default(), None, None).await;
    assert!(result.upgrade_id.is_none());
    assert_eq!(result.disruption, Disruption::Unspecified);
}

#[tokio::test]
async fn package_error_blocks_firmware() {
    let result = prepare(Vec::new(), Some("new"), async move {
        Err::<Option<PackageOffer>, _>("package check failed")
    })
    .await;
    assert_eq!(
        result.expect_err("BUG: package failure must propagate"),
        "package check failed"
    );
}

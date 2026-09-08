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
    offers
        .check(
            vec!["requested".to_owned()],
            async { Ok::<_, ()>(firmware) },
            |_| async { Ok(packages) },
        )
        .await
        .expect("BUG: fixture probes succeed")
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
    let offer = offers.claim(&result.upgrade_id.expect("BUG: firmware offer"));
    assert!(
        matches!(offer, Some(UpgradeOffer::Firmware { firmware: "firmware", package_preview: Some(packages), install }) if packages.bmc_version.as_deref() == Some("packages") && install == ["requested"])
    );
}

#[tokio::test]
async fn packages_only_retains_checked_payload_and_install() {
    let mut offers = UpgradeOfferCache::default();
    let result = check(&mut offers, None, Some(packages("checked-index"))).await;
    assert_eq!(result.disruption, Disruption::AppRestart);
    let offer = offers.claim(&result.upgrade_id.expect("BUG: package offer"));
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
async fn package_error_blocks_firmware_and_invalidates_old_offer() {
    let mut offers = UpgradeOfferCache::default();
    let old = check(&mut offers, Some("old"), None)
        .await
        .upgrade_id
        .expect("BUG: old offer");
    let result = offers
        .check(
            Vec::new(),
            async { Ok(Some("new")) },
            |estimate| async move {
                assert!(matches!(estimate, EstimateMode::Skip));
                Err::<Option<PackageOffer>, _>("package check failed")
            },
        )
        .await;
    assert_eq!(
        result.expect_err("BUG: package failure must propagate"),
        "package check failed"
    );
    assert!(offers.claim(&old).is_none());
}

#[tokio::test]
async fn firmware_failure_skips_package_probe() {
    let result = UpgradeOfferCache::<&str>::default()
        .check(
            Vec::new(),
            async { Err::<Option<&str>, _>("firmware failed") },
            |_| async {
                panic!("BUG: package probe must not run after firmware failure");
                #[expect(unreachable_code)]
                Ok(None::<PackageOffer>)
            },
        )
        .await;
    assert_eq!(
        result.expect_err("BUG: failure must propagate"),
        "firmware failed"
    );
}

#[tokio::test]
async fn package_only_check_requests_size_estimation() {
    UpgradeOfferCache::<&str>::default()
        .check(
            Vec::new(),
            async { Ok::<_, ()>(None) },
            |estimate| async move {
                assert!(matches!(estimate, EstimateMode::Estimate));
                Ok(Some(packages("packages")))
            },
        )
        .await
        .expect("BUG: fixture check succeeds");
}

#[tokio::test]
async fn claims_are_single_use_and_wrong_ids_preserve_current_offer() {
    let mut offers = UpgradeOfferCache::default();
    let id = check(&mut offers, Some("firmware"), None)
        .await
        .upgrade_id
        .expect("BUG: offer");
    assert!(offers.claim("unknown").is_none());
    assert!(offers.claim(&id).is_some());
    assert!(offers.claim(&id).is_none());
}

#[tokio::test]
async fn a_new_check_replaces_previous_offer() {
    let mut offers = UpgradeOfferCache::default();
    let old = check(&mut offers, Some("old"), None)
        .await
        .upgrade_id
        .expect("BUG: old offer");
    let new = check(&mut offers, Some("new"), None)
        .await
        .upgrade_id
        .expect("BUG: new offer");
    assert_ne!(old, new);
    assert!(offers.claim(&old).is_none());
    assert!(offers.claim(&new).is_some());
}

#[tokio::test]
async fn cancelled_check_does_not_restore_previous_offer() {
    let mut offers = UpgradeOfferCache::default();
    let old = check(&mut offers, Some("old"), None)
        .await
        .upgrade_id
        .expect("BUG: old offer");
    {
        let future = offers.check(
            Vec::new(),
            std::future::pending::<Result<Option<&str>, ()>>(),
            |_| async { Ok(None::<PackageOffer>) },
        );
        tokio::pin!(future);
        assert!(futures::poll!(&mut future).is_pending());
    }
    assert!(offers.claim(&old).is_none());
}

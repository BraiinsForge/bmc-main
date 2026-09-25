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

use bmc_nix::types::{InstalledBy, ManifestPackage, MergedPackageEntry};

impl Fingerprint for UpgradeOffer<&'static str> {
    fn fingerprint(&self) -> OfferFingerprint<'_> {
        match self {
            Self::Firmware {
                firmware, install, ..
            } => OfferFingerprint::Firmware {
                hash: firmware,
                install: Some(install_set(install)),
            },
            Self::Packages { packages, install } => OfferFingerprint::Packages {
                plan: packages.plan_digest(),
                install: install_set(install),
            },
        }
    }
}

fn packages(version: &str) -> PackageOffer {
    PackageOffer {
        index: MergedIndex {
            packages: Vec::new(),
            by_name: BTreeMap::new(),
        },
        manifest: Manifest::default(),
        preview: PackagesPreview {
            changes: Vec::new(),
            download_size_bytes: None,
            unpacked_size_bytes: None,
            bmc_version: Some(version.to_owned()),
            bmc_changelog: None,
        },
    }
}

fn indexed(name: &str) -> MergedPackageEntry {
    MergedPackageEntry {
        name: name.to_owned(),
        version: semver::Version::new(1, 0, 0),
        store_path: format!("/nix/store/{name}"),
        category: None,
        description: None,
        upgrade_strategy: None,
        install_strategy: None,
        server_id: "server".to_owned(),
        server_priority: 0,
        metadata: BTreeMap::new(),
    }
}

fn installed(name: &str) -> ManifestPackage {
    ManifestPackage {
        version: "1.0.0".to_owned(),
        store_path: format!("/nix/store/{name}"),
        category: None,
        description: None,
        upgrade_strategy: None,
        install_strategy: None,
        installed_by: InstalledBy::System,
        installed_from: "server".to_owned(),
        pinned: None,
    }
}

async fn check(
    offers: &mut UpgradeOfferCache<&'static str>,
    firmware: Option<&'static str>,
    packages: Option<PackageOffer>,
) -> OfferCheck<&'static str> {
    check_installing(offers, vec!["requested".to_owned()], firmware, packages).await
}

async fn check_installing(
    offers: &mut UpgradeOfferCache<&'static str>,
    install: Vec<String>,
    firmware: Option<&'static str>,
    packages: Option<PackageOffer>,
) -> OfferCheck<&'static str> {
    let prepared = prepare(install, firmware, async { Ok::<_, ()>(packages) })
        .await
        .expect("BUG: fixture probes succeed");
    offers.cache(prepared)
}

fn offer_id(check: &OfferCheck<&'static str>) -> ExecutionId {
    check.upgrade_id.expect("BUG: the check offers an upgrade")
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

#[tokio::test]
async fn a_check_that_finds_nothing_drops_the_offer() {
    let mut offers = UpgradeOfferCache::default();
    let id = offer_id(&check(&mut offers, Some("firmware"), None).await);
    assert!(check(&mut offers, None, None).await.upgrade_id.is_none());
    assert!(offers.claim(id).is_none());
}

#[tokio::test]
async fn a_newer_preview_keeps_the_firmware_id_and_is_the_one_claimed() {
    let mut offers = UpgradeOfferCache::default();
    let id = offer_id(&check(&mut offers, Some("firmware"), Some(packages("v1"))).await);
    let again = check(&mut offers, Some("firmware"), Some(packages("v2"))).await;
    assert_eq!(again.upgrade_id, Some(id));
    assert_eq!(
        again
            .packages
            .as_ref()
            .and_then(|preview| preview.bmc_version.as_deref()),
        Some("v2")
    );
    assert!(
        matches!(offers.claim(id), Some(UpgradeOffer::Firmware { package_preview: Some(preview), .. }) if preview.bmc_version.as_deref() == Some("v2"))
    );
}

#[tokio::test]
async fn a_different_package_index_retires_the_id() {
    let mut offers = UpgradeOfferCache::default();
    let id = offer_id(&check(&mut offers, None, Some(packages("same"))).await);
    assert_eq!(
        check(&mut offers, None, Some(packages("same")))
            .await
            .upgrade_id,
        Some(id)
    );
    let mut refreshed = packages("same");
    refreshed.index.packages.push(indexed("widget-weather"));
    let new = offer_id(&check(&mut offers, None, Some(refreshed)).await);
    assert_ne!(new, id);
    assert!(offers.claim(id).is_none());
    assert!(offers.claim(new).is_some());
}

#[tokio::test]
async fn a_different_installed_profile_retires_the_id() {
    let mut offers = UpgradeOfferCache::default();
    let id = offer_id(&check(&mut offers, None, Some(packages("same"))).await);
    let mut changed = packages("same");
    changed
        .manifest
        .packages
        .insert("widget-weather".to_owned(), installed("widget-weather"));
    let new = offer_id(&check(&mut offers, None, Some(changed)).await);
    assert_ne!(new, id);
    assert!(offers.claim(id).is_none());
}

#[tokio::test]
async fn install_order_and_duplicates_do_not_change_the_id() {
    for (firmware, packages) in [(Some("firmware"), None), (None, Some(packages("same")))] {
        let mut offers = UpgradeOfferCache::default();
        let mut id_for = async |install: &[&str]| {
            let install = install.iter().map(|name| (*name).to_owned()).collect();
            offer_id(&check_installing(&mut offers, install, firmware, packages.clone()).await)
        };
        let first = id_for(&["a"]).await;
        let second = id_for(&["a", "b"]).await;
        assert_ne!(second, first, "another package is another upgrade");
        assert_eq!(id_for(&["b", "a", "a"]).await, second);
        let Some(UpgradeOffer::Firmware { install, .. } | UpgradeOffer::Packages { install, .. }) =
            offers.claim(second)
        else {
            panic!("BUG: the kept id claims the offer");
        };
        assert_eq!(
            install,
            ["b", "a", "a"],
            "the run gets the latest request as sent"
        );
    }
}

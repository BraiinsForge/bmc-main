// Copyright (C) 2025 Braiins Systems s.r.o.

use crate::{BmcPlatform, BmcRelease};
use idxgen_data::bmc::v1::FileAsset;
use index_common::NormalizedIndex;
use itertools::Itertools;
use minerctl_defs::bos::version::{BosVersion, VersionName};
use std::cmp::Ordering;
use std::ops::Not;
use thiserror::Error;

/// A layer that implements various queries on top of the BMC index.
#[derive(Debug)]
pub struct IndexQueryLayer<'i> {
    index: &'i NormalizedIndex<BmcRelease>,
}

impl<'i> IndexQueryLayer<'i> {
    #[must_use]
    pub fn new(index: &'i NormalizedIndex<BmcRelease>) -> Self {
        Self { index }
    }

    pub fn available_releases(
        &self,
        platform: BmcPlatform,
    ) -> impl Iterator<Item = (&'i BmcRelease, &'i FileAsset)> + use<'i> {
        self.index.releases.iter().filter_map(move |release| {
            let asset = release.asset_for_platform(platform)?;

            Some((release, asset))
        })
    }

    /// Empty returned vector means that there are no upgrades to perform - the system is already up-to-date.
    pub fn upgrade_sequence(
        &self,
        platform: BmcPlatform,
        current_version: BosVersion,
    ) -> Result<Vec<BmcRelease>, UpgradeResolverError> {
        let current_version = current_version.version;

        let available_releases = self
            .available_releases(platform)
            .map(|(release, _)| release)
            .collect_vec();

        let available_versions = available_releases
            .iter()
            .map(|release| release.bmc_version.version)
            .collect_vec();

        let latest_version = available_versions
            .iter()
            .max()
            .ok_or(UpgradeResolverError::NoReleases)?;

        let upgrade_sequence =
            Self::resolve_upgrade_sequence(*latest_version, current_version, &available_releases)?;

        Ok(upgrade_sequence.into_iter().cloned().collect_vec())
    }

    /// Resolve an upgrade sequence for BMC sysupgrade. This function currently completely covers the available
    /// functionality of BMC sysupgrade.
    ///
    /// Contrary to its name, this function is capable of resolving both upgrades and downgrades. Downgrades are
    /// just negative upgrades :).
    ///
    /// Possible outputs:
    /// - `Err(_)`: requested upgrade couldn't be performed (the request is invalid or doesn't make sense)
    /// - `empty vec`: the upgrade is a "no-op" (no upgrade is required)
    /// - `vec.len() > 1`: upgrades should be performed in this order
    ///
    /// Note that `current_version` doesn't have to be included in `available_versions`, but it does have some
    /// implications regarding per partes upgrades if it isn't included.
    fn resolve_upgrade_sequence<'r>(
        target_version: VersionName,
        current_version: VersionName,
        available_releases: &'r [&BmcRelease],
    ) -> Result<Vec<&'r BmcRelease>, UpgradeResolverError> {
        // make sure that `available_versions` doesn't contain any duplicates
        // (duplicated versions would produce invalid output sequence)
        let duplicates = available_releases
            .iter()
            .map(|r| r.bmc_version.version)
            .duplicates()
            .collect_vec();
        if duplicates.is_empty().not() {
            return Err(UpgradeResolverError::AvailableVersionsDuplicates(
                duplicates,
            ));
        }

        let upgrade_sequence = match target_version.cmp(&current_version) {
            // no-op
            Ordering::Equal => Vec::new(),
            // upgrade
            Ordering::Greater => {
                // create per partes upgrade sequence
                available_releases
                    .iter()
                    .filter(|r| {
                        current_version < r.bmc_version.version
                            && r.bmc_version.version <= target_version
                    })
                    .filter(|r| r.is_major || r.bmc_version.version == target_version)
                    .copied()
                    .sorted_by_key(|r| r.bmc_version.version)
                    .collect_vec()
            }
            // downgrade
            Ordering::Less => {
                // downgrade is only possible within the current major version
                let crosses_major = available_releases
                    .iter()
                    .filter(|r| {
                        current_version >= r.bmc_version.version
                            && r.bmc_version.version > target_version
                    })
                    .any(|r| r.is_major);

                if crosses_major {
                    return Err(UpgradeResolverError::DowngradeCrossesMajor);
                }

                // downgrade is approved, we just have to reverse lookup Version from VersionName
                let ver = *available_releases
                    .iter()
                    .find(|r| r.bmc_version.version == target_version)
                    .expect("internal inconsistency");

                vec![ver]
            }
        };

        Ok(upgrade_sequence)
    }
}

#[derive(Error, Debug, Clone, PartialEq)]
pub enum UpgradeResolverError {
    #[error("no available releases")]
    NoReleases,
    #[error("duplicates in available versions: [{0:?}]")]
    AvailableVersionsDuplicates(Vec<VersionName>),
    #[error("downgrade within major version")]
    DowngradeCrossesMajor,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use minerctl_defs::commit::CommitHashShort;

    #[expect(non_snake_case)]
    #[test]
    fn upgrade_sequence() {
        fn make_release(number: u8, is_major: bool) -> BmcRelease {
            BmcRelease {
                bmc_version: BosVersion {
                    date: NaiveDate::from_ymd(2023, u32::from(number), 1),
                    day_index: 0,
                    commit: CommitHashShort::new(0),
                    version: VersionName {
                        year: 23,
                        month: number,
                        patch: None,
                    },
                    is_plus: true,
                    build: None,
                },
                is_major,
                release_date: NaiveDate::from_ymd(2023, u32::from(number), 1),
                description: String::new(),
                assets: None,
            }
        }

        let G = &make_release(7, false);
        let F = &make_release(6, false);
        let E = &make_release(5, true); // major
        let D = &make_release(4, false);
        let C = &make_release(3, true); // major
        let B = &make_release(2, false);
        let A = &make_release(1, false);

        let available_versions = &[A, B, C, D, E, F, G];

        let test_cases = &[
            // to latest
            ((A, G), Ok(vec![C, E, G])),
            ((B, G), Ok(vec![C, E, G])),
            ((C, G), Ok(vec![E, G])),
            ((D, G), Ok(vec![E, G])),
            ((E, G), Ok(vec![G])),
            ((F, G), Ok(vec![G])),
            // to major
            ((A, C), Ok(vec![C])),
            ((B, C), Ok(vec![C])),
            ((B, E), Ok(vec![C, E])),
            // across major
            ((A, D), Ok(vec![C, D])),
            ((B, F), Ok(vec![C, E, F])),
            // no-op
            ((E, E), Ok(vec![])),
            ((F, F), Ok(vec![])),
            ((G, G), Ok(vec![])),
            // downgrade
            ((G, F), Ok(vec![F])),
            ((G, E), Ok(vec![E])),
            ((F, E), Ok(vec![E])),
            ((D, C), Ok(vec![C])),
            // downgrade across major
            ((G, D), Err(UpgradeResolverError::DowngradeCrossesMajor)),
            ((G, C), Err(UpgradeResolverError::DowngradeCrossesMajor)),
            ((G, A), Err(UpgradeResolverError::DowngradeCrossesMajor)),
            ((F, B), Err(UpgradeResolverError::DowngradeCrossesMajor)),
            ((E, C), Err(UpgradeResolverError::DowngradeCrossesMajor)),
            ((E, A), Err(UpgradeResolverError::DowngradeCrossesMajor)),
            ((C, A), Err(UpgradeResolverError::DowngradeCrossesMajor)),
            ((E, D), Err(UpgradeResolverError::DowngradeCrossesMajor)),
        ];

        for ((current_version, target_version), expected_upgrade_sequence) in test_cases {
            let calculated_upgrade_sequence = IndexQueryLayer::resolve_upgrade_sequence(
                target_version.bmc_version.version,
                current_version.bmc_version.version,
                available_versions,
            );

            assert_eq!(
                expected_upgrade_sequence.as_ref().map(|x| x
                    .iter()
                    .map(|x| x.bmc_version.version.to_string())
                    .collect_vec()),
                calculated_upgrade_sequence.as_ref().map(|x| x
                    .iter()
                    .map(|x| x.bmc_version.version.to_string())
                    .collect_vec()),
                "incorrect upgrade sequence for {} --> {}",
                current_version.bmc_version.version,
                target_version.bmc_version.version,
            );
        }
    }
}

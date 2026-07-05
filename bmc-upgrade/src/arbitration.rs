// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::firmware::UpgradeDetail;
use crate::packages::PackagesPreview;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disruption {
    AppRestart,
    Reboot,
    Unspecified,
}

#[must_use]
pub fn arbitrate(
    firmware: Option<&UpgradeDetail>,
    packages: Option<&PackagesPreview>,
) -> Disruption {
    match (firmware, packages) {
        (Some(_), _) => Disruption::Reboot,
        (None, Some(_)) => Disruption::AppRestart,
        (None, None) => Disruption::Unspecified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::UpgradeMetadata;

    fn test_upgrade_detail() -> UpgradeDetail {
        UpgradeDetail {
            latest_release: UpgradeMetadata::new(
                "hash".to_owned(),
                "1.0.0".to_owned(),
                chrono::NaiveDate::default(),
                "description".to_owned(),
                "http://x".to_owned(),
                1,
            ),
            previous_releases: Vec::new(),
        }
    }

    fn test_packages_preview() -> PackagesPreview {
        PackagesPreview {
            changes: Vec::new(),
            download_size_bytes: None,
            bmc_version: None,
            bmc_changelog: None,
        }
    }

    #[test]
    fn arbitrate_firmware_wins_over_packages() {
        let firmware = test_upgrade_detail();
        let packages = test_packages_preview();
        assert_eq!(
            arbitrate(Some(&firmware), Some(&packages)),
            Disruption::Reboot
        );
    }

    #[test]
    fn arbitrate_packages_only_is_app_restart() {
        let packages = test_packages_preview();
        assert_eq!(arbitrate(None, Some(&packages)), Disruption::AppRestart);
    }

    #[test]
    fn arbitrate_nothing_available_is_unspecified() {
        assert_eq!(arbitrate(None, None), Disruption::Unspecified);
    }
}

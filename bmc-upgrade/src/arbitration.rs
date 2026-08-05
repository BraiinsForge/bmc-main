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
            unpacked_size_bytes: None,
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

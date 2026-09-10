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

use std::future::Future;

use bmc_nix::types::MergedIndex;
use bmc_upgrade_types::PackagesPreview;
use uuid::Uuid;

use crate::arbitration::Disruption;

#[derive(Clone, Debug)]
pub struct PackageOffer {
    pub index: MergedIndex,
    pub preview: PackagesPreview,
}

#[derive(Clone, Debug)]
pub enum UpgradeOffer<F> {
    Firmware {
        firmware: F,
        package_preview: Option<PackagesPreview>,
        install: Vec<String>,
    },
    Packages {
        packages: PackageOffer,
        install: Vec<String>,
    },
}

impl<F> UpgradeOffer<F> {
    #[must_use]
    pub fn package_preview(&self) -> Option<&PackagesPreview> {
        match self {
            Self::Firmware {
                package_preview, ..
            } => package_preview.as_ref(),
            Self::Packages { packages, .. } => Some(&packages.preview),
        }
    }
}

#[derive(Debug)]
pub struct OfferCheck<F> {
    pub firmware: Option<F>,
    pub packages: Option<PackagesPreview>,
    pub upgrade_id: Option<String>,
    pub disruption: Disruption,
}

#[derive(Debug)]
pub struct UpgradePreparation<F> {
    pub firmware: Option<F>,
    pub packages: Option<PackagesPreview>,
    pub upgrade: Option<UpgradeOffer<F>>,
    pub disruption: Disruption,
}

pub async fn prepare<F: Clone, E, PF>(
    install: Vec<String>,
    firmware: Option<F>,
    packages: PF,
) -> Result<UpgradePreparation<F>, E>
where
    PF: Future<Output = Result<Option<PackageOffer>, E>>,
{
    let packages = packages.await?;
    let packages_preview = packages.as_ref().map(|packages| packages.preview.clone());
    let (upgrade, disruption) = if let Some(firmware) = firmware.clone() {
        (
            Some(UpgradeOffer::Firmware {
                firmware,
                package_preview: packages_preview.clone(),
                install,
            }),
            Disruption::Reboot,
        )
    } else if let Some(packages) = packages {
        (
            Some(UpgradeOffer::Packages { packages, install }),
            Disruption::AppRestart,
        )
    } else {
        (None, Disruption::Unspecified)
    };
    Ok(UpgradePreparation {
        firmware,
        packages: packages_preview,
        upgrade,
        disruption,
    })
}

/// Callers hold their operation lock across checking or claiming and through execution, and
/// invalidate before preparing a new check so a failed or cancelled preparation leaves no stale
/// offer behind.
#[derive(Debug)]
pub struct UpgradeOfferCache<F> {
    current: Option<(String, UpgradeOffer<F>)>,
}

impl<F> Default for UpgradeOfferCache<F> {
    fn default() -> Self {
        Self { current: None }
    }
}

impl<F> UpgradeOfferCache<F> {
    pub fn invalidate(&mut self) {
        self.current = None;
    }

    pub fn cache(&mut self, prepared: UpgradePreparation<F>) -> OfferCheck<F> {
        self.current = prepared
            .upgrade
            .map(|offer| (Uuid::new_v4().to_string(), offer));
        let upgrade_id = self.current.as_ref().map(|(id, _)| id.clone());
        OfferCheck {
            firmware: prepared.firmware,
            packages: prepared.packages,
            upgrade_id,
            disruption: prepared.disruption,
        }
    }

    pub fn claim(&mut self, id: &str) -> Option<UpgradeOffer<F>> {
        if self
            .current
            .as_ref()
            .is_some_and(|(current, _)| current == id)
        {
            self.current.take().map(|(_, offer)| offer)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests;

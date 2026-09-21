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

use bmc_nix::gc::CollectGarbagePhase;
use bmc_nix::upgrade::UpgradePhase;
use bmc_upgrade_types::PackagePhase;

/// Maps a Nix upgrade phase onto the package phase reported to clients.
///
/// A free function, not a `From` impl: both types are foreign here, so the
/// orphan rule rejects the impl.
#[must_use]
pub fn package_phase(phase: UpgradePhase) -> PackagePhase {
    match phase {
        UpgradePhase::Realizing => PackagePhase::Realizing,
        UpgradePhase::Verifying => PackagePhase::Verifying,
        UpgradePhase::Building => PackagePhase::Building,
        UpgradePhase::Activating => PackagePhase::Activating,
        UpgradePhase::Cleaning => PackagePhase::Cleaning,
        UpgradePhase::CollectingGarbage(CollectGarbagePhase::FindingRoots) => {
            PackagePhase::FindingGarbageRoots
        }
        UpgradePhase::CollectingGarbage(CollectGarbagePhase::DeterminingLiveness) => {
            PackagePhase::DeterminingGarbageLiveness
        }
    }
}

#[cfg(test)]
mod tests;

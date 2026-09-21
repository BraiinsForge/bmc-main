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

use super::package_phase;

/// Every Nix phase has to reach the wire: one with no package counterpart
/// would leave the client's progress stuck on the phase before it.
#[test]
fn every_nix_phase_maps_to_a_package_phase() {
    let cases = [
        (UpgradePhase::Realizing, PackagePhase::Realizing),
        (UpgradePhase::Verifying, PackagePhase::Verifying),
        (UpgradePhase::Building, PackagePhase::Building),
        (UpgradePhase::Activating, PackagePhase::Activating),
        (UpgradePhase::Cleaning, PackagePhase::Cleaning),
        (
            UpgradePhase::CollectingGarbage(CollectGarbagePhase::FindingRoots),
            PackagePhase::FindingGarbageRoots,
        ),
        (
            UpgradePhase::CollectingGarbage(CollectGarbagePhase::DeterminingLiveness),
            PackagePhase::DeterminingGarbageLiveness,
        ),
    ];
    for (nix, expected) in cases {
        assert_eq!(package_phase(nix), expected);
    }
}

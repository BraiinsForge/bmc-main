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

//! Vendored `deck_upgrade_v1.xml` Wayland protocol: compositor-relayed upgrade
//! progress snapshots for the firmware and package overlay clients.

mod decoder;

use bmc_intl_message::{Message, message_descriptor};
pub use decoder::{DownloadProgress, UpgradeDecoder, UpgradeSnapshot, UpgradeState, UpgradeUpdate};

/// Split a byte count into the high and low words carried by the protocol.
#[must_use]
pub fn split_u64(value: u64) -> (u32, u32) {
    let high = u32::try_from(value >> 32).expect("BUG: upper u64 word must fit u32");
    let low = u32::try_from(value & u64::from(u32::MAX))
        .expect("BUG: masked lower u64 word must fit u32");
    (high, low)
}

/// Join the high and low protocol words into their original byte count.
#[must_use]
pub fn join_u64(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

impl server::deck_upgrade_v1::DeckUpgradeV1 {
    /// Send `download_progress`, splitting the byte count into wire words.
    pub fn send_download_progress(&self, downloaded_bytes: u64) {
        let (hi, lo) = split_u64(downloaded_bytes);
        self.download_progress(hi, lo);
    }

    /// Send `download_progress_with_total`, splitting each byte count into
    /// wire words.
    pub fn send_download_progress_with_total(&self, downloaded_bytes: u64, total_bytes: u64) {
        let (downloaded_hi, downloaded_lo) = split_u64(downloaded_bytes);
        let (total_hi, total_lo) = split_u64(total_bytes);
        self.download_progress_with_total(downloaded_hi, downloaded_lo, total_hi, total_lo);
    }
}

/// The titles live with the enum because it is `#[non_exhaustive]` outside this crate.
/// Only here can a match cover every phase without a wildcard,
/// and a phase added to the protocol XML is a compile error until it is named.
impl client::deck_upgrade_v1::Phase {
    /// The phase as an upgrade overlay titles it.
    #[must_use]
    pub fn title(self) -> Message {
        match self {
            Self::FirmwareDownloading => message_descriptor!(
                default_message: "Downloading firmware",
                description: "Upgrade overlay title during the firmware download",
            ),
            Self::FirmwareVerifying => message_descriptor!(
                default_message: "Verifying firmware",
                description: "Upgrade overlay title while the downloaded firmware is checked",
            ),
            Self::FirmwareApplying => message_descriptor!(
                default_message: "Applying firmware",
                description: "Upgrade overlay title while the firmware is written",
            ),
            Self::PackageRealizing => message_descriptor!(
                default_message: "Downloading packages",
                description: "Upgrade overlay title during the package download",
            ),
            Self::PackageVerifying => message_descriptor!(
                default_message: "Verifying packages",
                description: "Upgrade overlay title while the downloaded packages are checked",
            ),
            Self::PackageBuilding => message_descriptor!(
                default_message: "Building packages",
                description: "Upgrade overlay title while the packages are built",
            ),
            Self::PackageActivating => message_descriptor!(
                default_message: "Activating packages",
                description: "Upgrade overlay title while the new packages are switched in",
            ),
            Self::Rebooting => message_descriptor!(
                default_message: "Rebooting",
                description: "Upgrade overlay title while the device restarts into the upgrade",
            ),
        }
    }

    /// [`Self::title`] without its subject noun, for a surface too narrow
    /// to hold the title beside a percentage.
    #[must_use]
    pub fn short_title(self) -> Message {
        match self {
            Self::FirmwareDownloading | Self::PackageRealizing => message_descriptor!(
                default_message: "Downloading",
                description: "Upgrade overlay, short title before a percentage on a small display: downloading",
            ),
            Self::FirmwareVerifying | Self::PackageVerifying => message_descriptor!(
                default_message: "Verifying",
                description: "Upgrade overlay, short title before a percentage on a small display: verifying",
            ),
            Self::FirmwareApplying => message_descriptor!(
                default_message: "Applying",
                description: "Upgrade overlay, short title before a percentage on a small display: applying firmware",
            ),
            Self::PackageBuilding => message_descriptor!(
                default_message: "Building",
                description: "Upgrade overlay, short title before a percentage on a small display: building packages",
            ),
            Self::PackageActivating => message_descriptor!(
                default_message: "Activating",
                description: "Upgrade overlay, short title before a percentage on a small display: activating packages",
            ),
            Self::Rebooting => self.title(),
        }
    }
}

/// Server-side protocol bindings (for the compositor).
pub mod server {
    #![allow(
        unused_qualifications,
        clippy::all,
        clippy::pedantic,
        missing_debug_implementations
    )]

    use wayland_server;

    pub mod __interfaces {
        wayland_scanner::generate_interfaces!("./protocol/deck-upgrade-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("./protocol/deck-upgrade-v1.xml");
}

/// Client-side protocol bindings (for the overlay).
pub mod client {
    #![allow(
        unused_qualifications,
        clippy::all,
        clippy::pedantic,
        missing_debug_implementations
    )]

    use wayland_client;

    pub mod __interfaces {
        wayland_scanner::generate_interfaces!("./protocol/deck-upgrade-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("./protocol/deck-upgrade-v1.xml");
}

#[cfg(test)]
mod tests {
    use super::{join_u64, split_u64};
    use crate::client::deck_upgrade_v1::{Kind as ClientKind, Phase as ClientPhase};
    use crate::server::deck_upgrade_v1::{Kind as ServerKind, Phase as ServerPhase};

    #[test]
    fn u64_words_preserve_protocol_boundaries() {
        for (value, expected_words) in [
            (0, (0, 0)),
            (u64::from(u32::MAX), (0, u32::MAX)),
            (u64::from(u32::MAX) + 1, (1, 0)),
            (u64::MAX, (u32::MAX, u32::MAX)),
        ] {
            let (high, low) = split_u64(value);
            assert_eq!((high, low), expected_words, "must split {value}");
            assert_eq!(join_u64(high, low), value, "must preserve {value}");
        }
    }

    #[test]
    fn every_phase_has_a_title_and_a_short_one() {
        for phase in [
            ClientPhase::FirmwareDownloading,
            ClientPhase::FirmwareVerifying,
            ClientPhase::FirmwareApplying,
            ClientPhase::PackageRealizing,
            ClientPhase::PackageVerifying,
            ClientPhase::PackageBuilding,
            ClientPhase::PackageActivating,
            ClientPhase::Rebooting,
        ] {
            assert!(
                !phase.title().default_message.is_empty(),
                "{phase:?} must have a title"
            );
            assert!(
                !phase.short_title().default_message.is_empty(),
                "{phase:?} must have a short title"
            );
        }
    }

    #[test]
    fn generated_kind_enums_match_exact_wire_values_and_reject_unknown_values() {
        for (client, server, value) in [
            (ClientKind::Packages, ServerKind::Packages, 0),
            (ClientKind::Firmware, ServerKind::Firmware, 1),
        ] {
            assert_eq!(u32::from(client), value);
            assert_eq!(u32::from(server), value);
            assert_eq!(ClientKind::try_from(value), Ok(client));
            assert_eq!(ServerKind::try_from(value), Ok(server));
        }

        assert_eq!(ClientKind::try_from(2), Err(()));
        assert_eq!(ServerKind::try_from(2), Err(()));
    }

    #[test]
    fn generated_phase_enums_match_exact_wire_values_and_reject_unknown_values() {
        for (client, server, value) in [
            (
                ClientPhase::FirmwareDownloading,
                ServerPhase::FirmwareDownloading,
                0,
            ),
            (
                ClientPhase::FirmwareVerifying,
                ServerPhase::FirmwareVerifying,
                1,
            ),
            (
                ClientPhase::FirmwareApplying,
                ServerPhase::FirmwareApplying,
                2,
            ),
            (
                ClientPhase::PackageRealizing,
                ServerPhase::PackageRealizing,
                3,
            ),
            (
                ClientPhase::PackageVerifying,
                ServerPhase::PackageVerifying,
                4,
            ),
            (
                ClientPhase::PackageBuilding,
                ServerPhase::PackageBuilding,
                5,
            ),
            (
                ClientPhase::PackageActivating,
                ServerPhase::PackageActivating,
                6,
            ),
            (ClientPhase::Rebooting, ServerPhase::Rebooting, 7),
        ] {
            assert_eq!(u32::from(client), value);
            assert_eq!(u32::from(server), value);
            assert_eq!(ClientPhase::try_from(value), Ok(client));
            assert_eq!(ServerPhase::try_from(value), Ok(server));
        }

        assert_eq!(ClientPhase::try_from(8), Err(()));
        assert_eq!(ServerPhase::try_from(8), Err(()));
    }
}

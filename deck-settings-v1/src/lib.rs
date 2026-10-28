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

//! Vendored `deck_settings_v1` Wayland protocol: a compositor-relayed IPC
//! for the settings tray to control the system settings it shows.
//! The compositor hand-writes the server `Dispatch`; the overlay binds the
//! client side through `bmc-system-overlay`.

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
        wayland_scanner::generate_interfaces!("./protocol/deck-settings-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("./protocol/deck-settings-v1.xml");
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
        wayland_scanner::generate_interfaces!("./protocol/deck-settings-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("./protocol/deck-settings-v1.xml");
}

use bmc_intl_message::{Message, message_descriptor};

/// The sentences live with the enum because it is `#[non_exhaustive]` outside this crate.
/// Only here can a match cover every reason without a wildcard,
/// and a reason added to the protocol XML is a compile error until it is worded.
impl client::deck_settings_v1::DeclineReason {
    /// The reason as the settings tray words it,
    /// under the title of a declined restart.
    #[must_use]
    pub fn sentence(self) -> Message {
        match self {
            Self::UpgradeInProgress => message_descriptor!(
                default_message: "An upgrade is in progress",
                description: "Settings tray notice, under its title when the restart is declined: an upgrade is running",
            ),
            Self::RestartFailed => message_descriptor!(
                default_message: "The restart command failed",
                description: "Settings tray notice, under its title when the restart is declined: the device's reboot call failed",
            ),
        }
    }
}

impl server::deck_settings_v1::DeclineReason {
    /// The sentence for a tray bound below version 4,
    /// sent in English as `restart_declined` and shown as it came.
    #[must_use]
    pub fn english(self) -> &'static str {
        let reason = match self {
            Self::UpgradeInProgress => client::deck_settings_v1::DeclineReason::UpgradeInProgress,
            Self::RestartFailed => client::deck_settings_v1::DeclineReason::RestartFailed,
        };
        reason.sentence().default_message
    }
}

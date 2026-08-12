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

//! Wayland protocol extension for Deck widget communication.
//!
//! This crate provides generated Rust bindings for the `deck_widget_v1` Wayland protocol,
//! which enables communication between the compositor and widget processes.
//!
//! The protocol handles:
//! - Widget registration with instance ID
//! - Settings updates (timezone, localization, night mode)
//! - Automatic scene transition warm-up
//! - Graceful shutdown signaling
//! - Action requests (sound, LED control)

// The scanner macros read the XML during expansion,
// which rustc's dep-info never records — so a compile cache
// keyed on dep-info (CI's sccache) returns a stale object
// for an XML-only change.
//
// Observed as `wl_global_create: 2 > 1` in job 10001931
// after the version bump. `include_str!` puts the file's
// content into dep-info, so an XML change misses the cache
// like any source edit.
const _TRACK_PROTOCOL_XML: &str = include_str!("../protocol/deck-widget-v1.xml");

/// Server-side protocol bindings (for the compositor).
pub mod server {
    #![allow(
        unused_qualifications,
        clippy::all,
        clippy::pedantic,
        missing_debug_implementations
    )]

    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/deck-widget-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("./protocol/deck-widget-v1.xml");
}

/// Client-side protocol bindings (for widgets).
pub mod client {
    #![allow(
        unused_qualifications,
        clippy::all,
        clippy::pedantic,
        missing_debug_implementations
    )]

    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/deck-widget-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("./protocol/deck-widget-v1.xml");
}

mod types;

// Re-export common types for convenience
pub use bmc_shared_time::time::{DateFormat, TimeSystem, WeekDay};
pub use bmc_shared_utils::number_format::NumberFormat;
pub use bmc_shared_utils::temperature::TemperatureUnit;
pub use bmc_shared_utils::unit_system::UnitSystem;
pub use client::deck_widget_surface_v1::LifecycleState;
pub use types::{
    ActionPayload, CredentialSecrets, DeclaredSlot, DisplayInfo, DisplayShape, LED_REQUEST_ID_ALL,
    LedEffect, LedRequestId, LedRequestStatus, LedScope, Localization, NextAlarm, RgbColor,
    SecretsShapeError, SettingUpdate, Settings, ViewportShape, WidgetInitialConfig,
};
pub use wayland_client;

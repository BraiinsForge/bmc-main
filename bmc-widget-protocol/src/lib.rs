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
//! This crate provides generated Rust bindings for the `deck_widget` Wayland protocol,
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
const _TRACK_PROTOCOL_XML: &str = include_str!("../protocol/deck-widget.xml");

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
        wayland_scanner::generate_interfaces!("./protocol/deck-widget.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("./protocol/deck-widget.xml");
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
        wayland_scanner::generate_interfaces!("./protocol/deck-widget.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("./protocol/deck-widget.xml");
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
    LedEffect, LedRequestId, LedRequestStatus, LedScope, Localization, NextAlarm,
    ParseWidgetInstanceKeyError, RgbColor, SecretsShapeError, SettingUpdate, Settings,
    ViewportShape, WidgetInitialConfig, WidgetInstanceKey,
};
pub use wayland_client;

pub const BMC_WIDGET_KEY_ENV: &str = "BMC_WIDGET_KEY";

#[derive(Debug, thiserror::Error)]
pub enum WidgetKeyEnvError {
    #[error("{BMC_WIDGET_KEY_ENV} is not set")]
    Missing,
    #[error("{BMC_WIDGET_KEY_ENV} is not valid Unicode")]
    NotUnicode,
    #[error("{BMC_WIDGET_KEY_ENV} is not a canonical widget instance UUID")]
    Invalid(#[source] ParseWidgetInstanceKeyError),
}

fn parse_widget_key_env(
    value: Result<String, std::env::VarError>,
) -> Result<WidgetInstanceKey, WidgetKeyEnvError> {
    let value = match value {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Err(WidgetKeyEnvError::Missing),
        Err(std::env::VarError::NotUnicode(_)) => return Err(WidgetKeyEnvError::NotUnicode),
    };
    value.parse().map_err(WidgetKeyEnvError::Invalid)
}

/// Reads the canonical configured-widget UUID provided to a widget process.
///
/// # Errors
///
/// Returns an error if the variable is missing, not valid Unicode,
/// or not a canonical lowercase hyphenated UUID.
pub fn widget_key_from_env() -> Result<WidgetInstanceKey, WidgetKeyEnvError> {
    parse_widget_key_env(std::env::var(BMC_WIDGET_KEY_ENV))
}

#[cfg(test)]
mod widget_key_env_tests {
    use super::*;

    #[test]
    fn launch_key_must_exist_and_use_canonical_uuid_spelling() {
        assert!(matches!(
            parse_widget_key_env(Err(std::env::VarError::NotPresent)),
            Err(WidgetKeyEnvError::Missing)
        ));
        assert!(matches!(
            parse_widget_key_env(Ok("not-a-uuid".to_owned())),
            Err(WidgetKeyEnvError::Invalid(_))
        ));
        assert!(matches!(
            parse_widget_key_env(Ok("550E8400-E29B-41D4-A716-446655440000".to_owned())),
            Err(WidgetKeyEnvError::Invalid(_))
        ));

        let key = parse_widget_key_env(Ok("550e8400-e29b-41d4-a716-446655440000".to_owned()))
            .expect("BUG: canonical test UUID must parse from the launch environment");
        assert_eq!(key.to_string(), "550e8400-e29b-41d4-a716-446655440000");
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

//! Wayland protocol extension for Deck widget communication.
//!
//! This crate provides generated Rust bindings for the `deck_widget_v1` Wayland protocol,
//! which enables communication between the compositor and widget processes.
//!
//! The protocol handles:
//! - Widget registration with instance ID
//! - Settings updates (timezone, localization, night mode)
//! - Graceful shutdown signaling
//! - Action requests (sound, LED control)

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
pub use types::{
    ActionPayload, LED_REQUEST_ID_ALL, LedEffect, LedRequestId, LedRequestStatus, LedScope,
    Localization, RgbColor, SettingUpdate, Settings, SizeInfo, SizeType, WidgetInitialConfig,
};
pub use wayland_client;

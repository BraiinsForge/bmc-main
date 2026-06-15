// Copyright (C) 2026  Braiins Systems s.r.o.

//! Vendored `deck_settings_v1` Wayland protocol: a compositor-relayed IPC for
//! the settings-tray overlay to control display brightness and WiFi-setup mode.
//! The compositor hand-writes the server `Dispatch`; the overlay binds the
//! client side through `bmc-system-overlay`.

/// Server-side protocol bindings (for the compositor).
pub mod server {
    #![allow(
        unused_imports,
        unused_qualifications,
        clippy::all,
        clippy::pedantic,
        missing_debug_implementations
    )]

    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/deck-settings-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("./protocol/deck-settings-v1.xml");
}

/// Client-side protocol bindings (for the overlay).
pub mod client {
    #![allow(
        unused_imports,
        unused_qualifications,
        clippy::all,
        clippy::pedantic,
        missing_debug_implementations
    )]

    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/deck-settings-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("./protocol/deck-settings-v1.xml");
}

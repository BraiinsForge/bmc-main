// Copyright (C) 2026  Braiins Systems s.r.o.

//! Vendored `deck-alarm-v1.xml` Wayland protocol: lightweight compositor-relayed
//! IPC for the alarm overlay to show firing alarm and allow to dismiss or snooze it.

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
        wayland_scanner::generate_interfaces!("./protocol/deck-alarm-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("./protocol/deck-alarm-v1.xml");
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
        wayland_scanner::generate_interfaces!("./protocol/deck-alarm-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("./protocol/deck-alarm-v1.xml");
}

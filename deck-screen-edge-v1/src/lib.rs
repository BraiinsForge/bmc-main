// Copyright (C) 2026  Braiins Systems s.r.o.

//! Vendored `deck_screen_edge_v1` Wayland protocol, forked and renamed from
//! `kde-screen-edge-v1`. The surface is hidden by default and holds no buffer
//! while hidden; the added `revealed`/`hidden` events drive allocation and
//! release. The compositor hand-writes the server `Dispatch`; overlays bind the
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
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/deck-screen-edge-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("./protocol/deck-screen-edge-v1.xml");
}

/// Client-side protocol bindings (for overlays).
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
        wayland_scanner::generate_interfaces!("./protocol/deck-screen-edge-v1.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("./protocol/deck-screen-edge-v1.xml");
}

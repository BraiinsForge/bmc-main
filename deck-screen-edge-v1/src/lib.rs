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

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

//! Server state and dispatch for the `deck_platform_v1` protocol.
//!
//! The capability set is fixed at compositor start and sent once per bind,
//! so there is nothing to replay later and no resource list to keep.

use ::deck_platform_v1::server::deck_platform_v1::{self, Capability, DeckPlatformV1};
use bmc_platform::HardwareCapabilities;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
};

use super::state::CompositorState;

/// The wire bitfield for a hardware profile's capability set.
#[must_use]
pub fn caps_wire(caps: HardwareCapabilities) -> Capability {
    let mut wire = Capability::empty();
    wire.set(Capability::Wifi, caps.wifi_supported);
    wire.set(Capability::Ethernet, caps.ethernet_supported);
    wire.set(Capability::Mining, caps.mining_supported);
    wire.set(Capability::BoserManaged, caps.boser_managed);
    wire
}

/// The capability set every `deck_platform_v1` bind is told.
#[derive(Debug, Clone, Copy)]
pub struct PlatformState {
    pub caps: Capability,
}

impl PlatformState {
    #[must_use]
    pub fn new(caps: HardwareCapabilities) -> Self {
        Self {
            caps: caps_wire(caps),
        }
    }
}

impl GlobalDispatch<DeckPlatformV1, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckPlatformV1>,
        (): &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let resource = data_init.init(resource, ());
        resource.capabilities(state.platform.caps);
    }
}

impl Dispatch<DeckPlatformV1, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &DeckPlatformV1,
        request: deck_platform_v1::Request,
        (): &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            deck_platform_v1::Request::Destroy => {}
            other => tracing::warn!("Unknown deck_platform_v1 request: {other:?}"),
        }
    }
}

/// Advertise the `deck_platform_v1` global.
pub fn create_global(display: &DisplayHandle) {
    display.create_global::<CompositorState, DeckPlatformV1, ()>(1, ());
}

#[cfg(test)]
mod tests {
    use bmc_platform::{HardwareProfile, Product};

    use super::*;

    fn wire_for(product: Product) -> Capability {
        caps_wire(HardwareProfile::for_product(product).capabilities())
    }

    #[test]
    fn deck_has_wifi_and_nothing_mining_related() {
        let deck = wire_for(Product::Bmc100);
        assert_eq!(deck, Capability::Wifi);
    }

    #[test]
    fn miner_boards_advertise_mining_and_boser() {
        for product in [Product::Bmm100, Product::Bmm101, Product::Bfm100] {
            let caps = wire_for(product);
            assert!(caps.contains(Capability::Mining), "{product:?}");
            assert!(caps.contains(Capability::BoserManaged), "{product:?}");
            assert!(caps.contains(Capability::Ethernet), "{product:?}");
        }
        assert!(!wire_for(Product::Bmm100).contains(Capability::Wifi));
        assert!(wire_for(Product::Bmm101).contains(Capability::Wifi));
    }
}

/// Drives a real in-process Wayland client/server handshake
/// so the bind is checked as a client sees it:
/// the capability event arrives, and carries the profile's bits.
#[cfg(test)]
mod bind_wire_test {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use ::deck_platform_v1::client::deck_platform_v1::{self as client_api, Capability};
    use bmc_platform::{HardwareProfile, Product};
    use smithay::reexports::wayland_server::Display;
    use wayland_client::protocol::wl_registry;
    use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};

    use crate::compositor::state::{ClientState, CompositorState};

    #[derive(Default)]
    struct TestClient {
        platform: Option<client_api::DeckPlatformV1>,
        seen: Vec<u32>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            (): &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
                && interface == "deck_platform_v1"
            {
                state.platform = Some(registry.bind::<client_api::DeckPlatformV1, _, _>(
                    name,
                    version.min(1),
                    qh,
                    (),
                ));
            }
        }
    }

    impl Dispatch<client_api::DeckPlatformV1, ()> for TestClient {
        fn event(
            state: &mut Self,
            _: &client_api::DeckPlatformV1,
            event: client_api::Event,
            (): &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                client_api::Event::Capabilities { capabilities } => {
                    state.seen.push(match capabilities {
                        WEnum::Value(caps) => caps.bits(),
                        WEnum::Unknown(raw) => raw,
                    });
                }
                other => panic!("BUG: unexpected deck_platform_v1 event {other:?}"),
            }
        }
    }

    fn compositor(product: Product) -> (Display<CompositorState>, CompositorState) {
        let display: Display<CompositorState> =
            Display::new().expect("BUG: test Wayland display should initialize");
        let compositor = CompositorState::new(
            &display,
            480,
            1280,
            480,
            1280,
            60_000,
            "test-seat",
            &HardwareProfile::for_product(product),
        );
        (display, compositor)
    }

    fn bind_and_collect(product: Product) -> Vec<u32> {
        let (mut display, mut compositor) = compositor(product);
        let (server_stream, client_stream) =
            UnixStream::pair().expect("BUG: unix socket pair should be creatable");
        display
            .handle()
            .insert_client(server_stream, Arc::new(ClientState::default()))
            .expect("BUG: test client stream should be insertable into a fresh display");

        let conn = Connection::from_socket(client_stream)
            .expect("BUG: test client socket should form a valid connection");
        let mut queue: EventQueue<TestClient> = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = TestClient::default();

        conn.display().get_registry(&qh, ());
        pump(
            &mut display,
            &mut compositor,
            &conn,
            &mut queue,
            &mut client,
        );
        assert!(
            client.platform.is_some(),
            "BUG: deck_platform_v1 global should have been advertised"
        );
        // The bind itself, then the one event the server answers it with.
        pump(
            &mut display,
            &mut compositor,
            &conn,
            &mut queue,
            &mut client,
        );

        client.seen
    }

    fn pump(
        display: &mut Display<CompositorState>,
        compositor: &mut CompositorState,
        conn: &Connection,
        queue: &mut EventQueue<TestClient>,
        client: &mut TestClient,
    ) {
        conn.flush()
            .expect("BUG: test client flush should succeed on a live socket pair");
        display
            .dispatch_clients(compositor)
            .expect("BUG: test server dispatch should succeed on a live socket pair");
        display
            .flush_clients()
            .expect("BUG: test server flush should succeed on a live socket pair");
        queue
            .blocking_dispatch(client)
            .expect("BUG: test client dispatch should succeed once the server has replied");
    }

    #[test]
    fn a_bind_on_a_miner_is_told_it_mines() {
        let seen = bind_and_collect(Product::Bmm101);
        assert_eq!(seen.len(), 1, "exactly one capabilities event: {seen:?}");
        let caps = Capability::from_bits_truncate(seen[0]);
        assert!(caps.contains(Capability::Mining));
        assert!(caps.contains(Capability::Wifi));
    }

    #[test]
    fn a_bind_on_a_deck_is_not_told_it_mines() {
        let seen = bind_and_collect(Product::Bmc100);
        assert_eq!(seen.len(), 1, "exactly one capabilities event: {seen:?}");
        let caps = Capability::from_bits_truncate(seen[0]);
        assert!(!caps.contains(Capability::Mining));
        assert!(caps.contains(Capability::Wifi));
    }
}

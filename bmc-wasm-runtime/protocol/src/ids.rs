// Copyright (C) 2026  Braiins Systems s.r.o.

//! Opaque newtype IDs for registered host-side resources.
//!
//! Prevents accidentally passing the wrong kind of ID to the wrong call
//! (e.g. a bitmap ID where an SVG ID is expected, or a WebSocket ID where
//! a socket ID is expected). Absent IDs are modelled as `Option<*Id>`,
//! never as a magic in-band sentinel.
//!
//! Wire format: each ID encodes as its inner integer. Zero is reserved as
//! the absent sentinel — `from_wire(0)` returns `None`, `to_wire(None)`
//! writes `0`. Outside the wire/FFI seam, prefer `Option<*Id>` over the
//! raw value.

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal, $inner:ty) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name($inner);

        impl $name {
            #[must_use]
            pub const fn from_wire(raw: $inner) -> Option<Self> {
                if raw == 0 { None } else { Some(Self(raw)) }
            }

            #[must_use]
            pub const fn to_wire(self) -> $inner {
                self.0
            }

            /// Bump a registry counter and return the fresh ID. Counter
            /// must start at `1`; panics on overflow or zero start.
            #[must_use]
            pub fn alloc(counter: &mut $inner) -> Self {
                let raw = *counter;
                assert_ne!(
                    raw, 0,
                    concat!("BUG: ", $kind, " allocator counter is 0; must start at 1"),
                );
                *counter = counter
                    .checked_add(1)
                    .unwrap_or_else(|| panic!(concat!("BUG: ", $kind, " ID space exhausted")));
                Self(raw)
            }
        }
    };
}

// ── Renderer resources (u16 wire) ───────────────────────────────────

define_id! {
    /// Opaque handle returned by SVG registration.
    SvgId, "svg", u16
}

define_id! {
    /// Opaque handle returned by bitmap registration.
    BitmapId, "bitmap", u16
}

define_id! {
    /// Opaque handle returned by mesh registration.
    MeshId, "mesh", u16
}

define_id! {
    /// Opaque handle returned by audio registration.
    AudioId, "audio", u16
}

// ── Network / data resources (u32 wire — wasmi FFI calling convention) ─

define_id! {
    /// Outbound HTTP fetch request handle, used to correlate fetch responses.
    FetchRequestId, "fetch request", u32
}

define_id! {
    /// Inbound HTTP listener request handle, used to address `host_http_respond`.
    HttpRequestId, "http request", u32
}

define_id! {
    /// WebSocket connection handle.
    WebsocketId, "websocket", u32
}

define_id! {
    /// TCP / TLS socket handle.
    SocketId, "socket", u32
}

define_id! {
    /// Parsed JSON document handle.
    JsonId, "json document", u32
}

define_id! {
    /// Parsed XML document handle.
    XmlId, "xml document", u32
}

define_id! {
    /// mDNS browse-session handle.
    MdnsBrowseId, "mdns browse", u32
}

define_id! {
    /// mDNS service-registration handle.
    MdnsRegId, "mdns registration", u32
}

define_id! {
    /// SSDP search-session handle.
    SsdpSearchId, "ssdp search", u32
}

define_id! {
    /// UDP broadcast-session handle.
    UdpBroadcastId, "udp broadcast", u32
}

define_id! {
    /// HTTP-listener handle (server side, accepting inbound requests).
    HttpListenerId, "http listener", u32
}

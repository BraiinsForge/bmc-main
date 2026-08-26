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

#[cfg(any(feature = "id-pool", test))]
use std::fmt;

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

/// FFI bridge for the u16 renderer ids.
/// They are u16 in storage and in the render-tree wire format
/// (compact — 2 bytes per draw node), but wasm has no integer type
/// narrower than i32, so they cross host imports/exports as u32.
/// `to_ffi` widens (free); `from_ffi` narrows + range-checks.
macro_rules! u16_ffi_bridge {
    ($($name:ident),+ $(,)?) => { $(
        impl $name {
            #[must_use]
            pub fn to_ffi(self) -> u32 {
                u32::from(self.to_wire())
            }

            #[must_use]
            pub fn from_ffi(raw: u32) -> Option<Self> {
                u16::try_from(raw).ok().and_then(Self::from_wire)
            }
        }
    )+ };
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

u16_ffi_bridge!(SvgId, BitmapId, MeshId, AudioId);

#[cfg(any(feature = "id-pool", test))]
mod reusable_id {
    pub trait Sealed {}
}

#[cfg(any(feature = "id-pool", test))]
#[doc(hidden)]
pub trait ReusableId: reusable_id::Sealed {}

/// Host-side pool that reuses released IDs before extending the high-water mark.
#[cfg(any(feature = "id-pool", test))]
pub struct IdPool<I: ReusableId> {
    next: u16,
    free: Vec<I>,
    exclusive_cap: u16,
}

#[cfg(any(feature = "id-pool", test))]
impl<I: ReusableId> IdPool<I> {
    #[must_use]
    pub fn new(exclusive_cap: u16) -> Self {
        Self {
            next: 1,
            free: Vec::new(),
            exclusive_cap,
        }
    }
}

#[cfg(any(feature = "id-pool", test))]
macro_rules! reusable_id {
    ($($id:ty),+ $(,)?) => {
        $(
            impl reusable_id::Sealed for $id {}

            impl ReusableId for $id {}

            impl IdPool<$id> {
                pub fn alloc(&mut self) -> Option<$id> {
                    if let Some(id) = self.free.pop() {
                        return Some(id);
                    }
                    if self.next >= self.exclusive_cap {
                        return None;
                    }
                    let id = <$id>::from_wire(self.next)
                        .expect("BUG: reusable ID pool issued zero");
                    self.next += 1;
                    Some(id)
                }

                pub fn release(&mut self, id: $id) {
                    let raw = id.to_wire();
                    assert!(
                        raw < self.next,
                        "BUG: released ID must have been issued by this pool"
                    );
                    debug_assert!(
                        self.free.iter().all(|free| free.to_wire() != raw),
                        "BUG: released ID must not already be free"
                    );
                    self.free.push(id);
                }
            }
        )+
    };
}

#[cfg(any(feature = "id-pool", test))]
reusable_id!(AudioId, BitmapId, SvgId);

#[cfg(any(feature = "id-pool", test))]
impl<I: ReusableId> fmt::Debug for IdPool<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdPool")
            .field("next", &self.next)
            .field("free", &self.free.len())
            .field("exclusive_cap", &self.exclusive_cap)
            .finish()
    }
}

// ── Network / data resources (u32 wire — wasmi FFI calling convention) ─

define_id! {
    /// Outbound HTTP fetch request handle, used to correlate fetch responses.
    FetchRequestId, "fetch request", u32
}

define_id! {
    /// Off-thread image-decode job handle, used to correlate `__on_image_ready`.
    ImageJobId, "image decode job", u32
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_ids_are_reused_before_fresh_ids() {
        let mut pool = IdPool::<BitmapId>::new(4);
        let first = pool.alloc().expect("BUG: first ID must fit");
        let second = pool.alloc().expect("BUG: second ID must fit");

        pool.release(first);

        assert_eq!(pool.alloc(), Some(first));
        assert_eq!(pool.alloc().map(BitmapId::to_wire), Some(3));
        assert_eq!(pool.alloc(), None);
        pool.release(second);
        assert_eq!(pool.alloc(), Some(second));
    }

    #[test]
    #[should_panic(expected = "BUG: released ID must not already be free")]
    fn releasing_an_id_twice_fails_loudly() {
        let mut pool = IdPool::<BitmapId>::new(4);
        let id = pool.alloc().expect("BUG: first ID must fit");

        pool.release(id);
        pool.release(id);
    }

    #[test]
    #[should_panic(expected = "BUG: released ID must have been issued by this pool")]
    fn releasing_an_unissued_id_fails_loudly() {
        let mut pool = IdPool::<BitmapId>::new(4);
        let unissued = BitmapId::from_wire(1).expect("BUG: nonzero bitmap ID must be valid");

        pool.release(unissued);
    }
}

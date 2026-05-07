// Copyright (C) 2026  Braiins Systems s.r.o.

//! Opaque newtype IDs for registered renderer resources.
//!
//! Prevents accidentally passing the wrong kind of ID to the wrong draw
//! function (e.g. a bitmap ID where an icon ID is expected). Absent IDs are
//! modelled as `Option<*Id>`, never as a magic in-band sentinel.
//!
//! Wire format: each ID encodes as a `u16`. Zero is reserved as the absent
//! sentinel — `from_wire(0)` returns `None`, `to_wire(None)` writes `0`.
//! Outside the wire/FFI seam, prefer `Option<*Id>` over the raw value.

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(u16);

        impl $name {
            #[must_use]
            pub const fn from_wire(raw: u16) -> Option<Self> {
                if raw == 0 { None } else { Some(Self(raw)) }
            }

            #[must_use]
            pub const fn to_wire(self) -> u16 {
                self.0
            }

            /// Bump a registry counter and return the fresh ID. Counter
            /// must start at `1`; panics on overflow or zero start.
            #[must_use]
            pub fn alloc(counter: &mut u16) -> Self {
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

define_id! {
    /// Opaque handle returned by icon registration.
    IconId, "icon"
}

define_id! {
    /// Opaque handle returned by bitmap registration.
    BitmapId, "bitmap"
}

define_id! {
    /// Opaque handle returned by mesh registration.
    MeshId, "mesh"
}

define_id! {
    /// Opaque handle returned by audio registration.
    AudioId, "audio"
}

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

//! Opaque newtype IDs for registered renderer resources.
//!
//! Prevents accidentally passing the wrong kind of ID to the wrong call
//! (e.g. a bitmap ID where an SVG ID is expected).
//! Absent IDs are modelled as `Option<*Id>`, never as a magic in-band sentinel.
//!
//! Wire format: each ID encodes as its inner integer.
//! Zero is reserved as the absent sentinel —
//! `from_wire(0)` returns `None`, `to_wire(None)` writes `0`.
//! Outside the wire/FFI seam, prefer `Option<*Id>` over the raw value.

/// Declare an opaque resource ID newtype with the zero-is-absent wire convention.
#[doc(hidden)]
#[macro_export]
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
///
/// They are u16 in storage and in the render-tree wire format (compact — 2 bytes per draw node),
/// but wasm has no integer type narrower than i32, so they cross host imports/exports as u32.
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

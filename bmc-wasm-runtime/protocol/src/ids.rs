// Copyright (C) 2026  Braiins Systems s.r.o.

//! Opaque newtype IDs for registered renderer resources.
//!
//! Prevents accidentally passing the wrong kind of ID to the wrong draw function
//! (e.g. a bitmap ID where an icon ID is expected).

/// Opaque handle returned by icon registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IconId(u16);

impl IconId {
    /// Sentinel value meaning "no icon".
    pub const NONE: Self = Self(0);

    /// Wrap a raw ID. Only registration functions should call this.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    /// Get the raw ID for FFI or serialization.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }
}

/// Opaque handle returned by bitmap registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitmapId(u16);

impl BitmapId {
    /// Sentinel value meaning "no bitmap".
    pub const NONE: Self = Self(0);

    /// Wrap a raw ID. Only registration functions should call this.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    /// Get the raw ID for FFI or serialization.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }
}

/// Opaque handle returned by mesh registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshId(u16);

impl MeshId {
    /// Sentinel value meaning "no mesh".
    pub const NONE: Self = Self(0);

    /// Wrap a raw ID. Only registration functions should call this.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    /// Get the raw ID for FFI or serialization.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }
}

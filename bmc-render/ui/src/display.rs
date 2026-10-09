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

//! Shape wire enums, shared between the WASM SDK (guest) and the runtime
//! (host). The `u32` reprs are the host↔wasm ABI for viewport/display info.

/// Visible display shape. `repr(u32)` is the wire value packed by the host
/// in `host_display_info` and decoded by the SDK's `display_info()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum DisplayShape {
    Rectangular = 0,
    Round = 1,
}

/// Drawable viewport shape. This is the widget-facing shape signal from the
/// Wayland `configure(..., viewport_shape)` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ViewportShape {
    Rectangular = 0,
    Round = 1,
}

/// Error from decoding an out-of-range `DisplayShape` wire value. A plain
/// struct (no `thiserror`) since this crate is shared with the wasm guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownDisplayShape(pub u32);

/// Error from decoding an out-of-range `ViewportShape` wire value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownViewportShape(pub u32);

impl TryFrom<u32> for DisplayShape {
    type Error = UnknownDisplayShape;

    /// Decode a wire `u32`. An out-of-range value is a host↔wasm ABI bug and
    /// is rejected, not silently coerced.
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Rectangular),
            1 => Ok(Self::Round),
            other => Err(UnknownDisplayShape(other)),
        }
    }
}

impl TryFrom<u32> for ViewportShape {
    type Error = UnknownViewportShape;

    /// Decode a wire `u32`. An out-of-range value is a host↔wasm ABI bug and
    /// is rejected, not silently coerced.
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Rectangular),
            1 => Ok(Self::Round),
            other => Err(UnknownViewportShape(other)),
        }
    }
}

impl From<DisplayShape> for u32 {
    fn from(shape: DisplayShape) -> Self {
        shape as Self
    }
}

impl From<ViewportShape> for u32 {
    fn from(shape: ViewportShape) -> Self {
        shape as Self
    }
}

#[cfg(feature = "widget-protocol")]
mod widget_protocol_conversions {
    use super::{DisplayShape, ViewportShape};

    impl From<bmc_widget_protocol::DisplayShape> for DisplayShape {
        fn from(shape: bmc_widget_protocol::DisplayShape) -> Self {
            match shape {
                bmc_widget_protocol::DisplayShape::Rectangular => Self::Rectangular,
                bmc_widget_protocol::DisplayShape::Round => Self::Round,
            }
        }
    }

    impl From<DisplayShape> for bmc_widget_protocol::DisplayShape {
        fn from(shape: DisplayShape) -> Self {
            match shape {
                DisplayShape::Rectangular => Self::Rectangular,
                DisplayShape::Round => Self::Round,
            }
        }
    }

    impl From<bmc_widget_protocol::ViewportShape> for ViewportShape {
        fn from(shape: bmc_widget_protocol::ViewportShape) -> Self {
            match shape {
                bmc_widget_protocol::ViewportShape::Rectangular => Self::Rectangular,
                bmc_widget_protocol::ViewportShape::Round => Self::Round,
            }
        }
    }

    impl From<ViewportShape> for bmc_widget_protocol::ViewportShape {
        fn from(shape: ViewportShape) -> Self {
            match shape {
                ViewportShape::Rectangular => Self::Rectangular,
                ViewportShape::Round => Self::Round,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{DisplayShape, ViewportShape};

        #[test]
        fn display_shape_roundtrips_through_widget_protocol() {
            for shape in [DisplayShape::Rectangular, DisplayShape::Round] {
                let wire: bmc_widget_protocol::DisplayShape = shape.into();
                assert_eq!(DisplayShape::from(wire), shape);
            }
        }

        #[test]
        fn viewport_shape_roundtrips_through_widget_protocol() {
            for shape in [ViewportShape::Rectangular, ViewportShape::Round] {
                let wire: bmc_widget_protocol::ViewportShape = shape.into();
                assert_eq!(ViewportShape::from(wire), shape);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DisplayShape, UnknownDisplayShape, UnknownViewportShape, ViewportShape};

    #[test]
    fn display_shape_wire_roundtrips() {
        for shape in [DisplayShape::Rectangular, DisplayShape::Round] {
            assert_eq!(DisplayShape::try_from(u32::from(shape)), Ok(shape));
        }
    }

    #[test]
    fn unknown_wire_value_is_rejected() {
        assert_eq!(DisplayShape::try_from(7), Err(UnknownDisplayShape(7)));
        assert_eq!(ViewportShape::try_from(7), Err(UnknownViewportShape(7)));
    }

    #[test]
    fn viewport_shape_wire_roundtrips() {
        for shape in [ViewportShape::Rectangular, ViewportShape::Round] {
            assert_eq!(ViewportShape::try_from(u32::from(shape)), Ok(shape));
        }
    }
}

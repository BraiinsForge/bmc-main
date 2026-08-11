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

//! Which platforms the host tools can preview and capture, and at which viewport geometries.
//!
//! Hardware facts — display size, shape, DPI, slot grid, LED strip — are not restated here;
//! they come from [`bmc_platform::HardwareProfile`], the same source the device itself uses.
//! What this module adds is preview-specific: the geometries a widget may occupy,
//! and the `<platform>:<viewport>` vocabulary that names one of them.

use std::fmt;
use std::str::FromStr;

use bmc_platform::HardwareProfile;
pub use bmc_platform::{DisplayProfile, DisplayShape, Product, SlotGrid};

/// How many slots of the platform's grid a viewport occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSpan {
    pub columns: usize,
    pub rows: usize,
}

/// Where a viewport sits on the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Fullscreen,
    SlotSpan(SlotSpan),
}

/// A geometry a widget can be previewed and captured at.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    /// Slug used in target strings, config keys and output paths.
    pub id: &'static str,
    pub label: &'static str,
    pub placement: Placement,
    pub shape: DisplayShape,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    #[must_use]
    pub fn runtime_viewport_shape(&self) -> bmc_wasm_protocol::ViewportShape {
        runtime_viewport_shape(self.shape)
    }
}

/// The viewport shape a guest sees through `widget_viewport()`.
#[must_use]
pub fn runtime_viewport_shape(shape: DisplayShape) -> bmc_wasm_protocol::ViewportShape {
    match shape {
        DisplayShape::Rectangular => bmc_wasm_protocol::ViewportShape::Rectangular,
        DisplayShape::Round => bmc_wasm_protocol::ViewportShape::Round,
    }
}

/// A previewable platform: a product plus the viewports widgets may occupy on it.
#[derive(Debug, Clone, Copy)]
pub struct Platform {
    /// Slug used in target strings, config keys and output paths.
    pub id: &'static str,
    /// Short name for the UI — [`Product::display_name`] is not unique,
    /// since BMM100 and BMM101 share "Mini Miner".
    pub label: &'static str,
    pub product: Product,
    pub viewports: &'static [Viewport],
}

impl Platform {
    #[must_use]
    pub fn hardware(&self) -> HardwareProfile {
        HardwareProfile::for_product(self.product)
    }

    #[must_use]
    pub fn display(&self) -> DisplayProfile {
        self.hardware().display
    }

    #[must_use]
    pub fn slot_grid(&self) -> Option<SlotGrid> {
        self.hardware().slot_grid
    }

    #[must_use]
    pub fn led_count(&self) -> Option<usize> {
        self.hardware().led_strip.map(|strip| strip.led_count)
    }

    /// The display geometry a guest sees through `display_info()`.
    #[must_use]
    pub fn runtime_display_info(&self) -> crate::RuntimeDisplayInfo {
        let display = self.display();
        crate::RuntimeDisplayInfo {
            width: display.logical_width,
            height: display.logical_height,
            shape: match display.shape {
                DisplayShape::Rectangular => bmc_wasm_protocol::DisplayShape::Rectangular,
                DisplayShape::Round => bmc_wasm_protocol::DisplayShape::Round,
            },
            dpi: display.dpi,
        }
    }

    #[must_use]
    pub fn viewport(&self, id: &str) -> Option<&'static Viewport> {
        self.viewports
            .iter()
            .find(|v| v.id.eq_ignore_ascii_case(id))
    }
}

const BMC100_VIEWPORTS: &[Viewport] = &[
    Viewport {
        id: "full",
        label: "Fullscreen",
        placement: Placement::Fullscreen,
        shape: DisplayShape::Rectangular,
        width: 1_280,
        height: 480,
    },
    Viewport {
        id: "large",
        label: "Large",
        placement: Placement::SlotSpan(SlotSpan {
            columns: 2,
            rows: 2,
        }),
        shape: DisplayShape::Rectangular,
        width: 638,
        height: 480,
    },
    Viewport {
        id: "medium",
        label: "Medium",
        placement: Placement::SlotSpan(SlotSpan {
            columns: 2,
            rows: 1,
        }),
        shape: DisplayShape::Rectangular,
        width: 638,
        height: 238,
    },
    Viewport {
        id: "small",
        label: "Small",
        placement: Placement::SlotSpan(SlotSpan {
            columns: 1,
            rows: 1,
        }),
        shape: DisplayShape::Rectangular,
        width: 317,
        height: 238,
    },
];

const BMM100_VIEWPORTS: &[Viewport] = &[Viewport {
    id: "full",
    label: "Fullscreen",
    placement: Placement::Fullscreen,
    shape: DisplayShape::Rectangular,
    width: 320,
    height: 240,
}];

const BMM101_VIEWPORTS: &[Viewport] = &[Viewport {
    id: "full",
    label: "Fullscreen",
    placement: Placement::Fullscreen,
    shape: DisplayShape::Rectangular,
    width: 480,
    height: 320,
}];

const BFM100_VIEWPORTS: &[Viewport] = &[Viewport {
    id: "full",
    label: "Fullscreen",
    placement: Placement::Fullscreen,
    shape: DisplayShape::Round,
    width: 480,
    height: 480,
}];

pub static PLATFORMS: &[Platform] = &[
    Platform {
        id: "bmc100",
        label: "Deck",
        product: Product::Bmc100,
        viewports: BMC100_VIEWPORTS,
    },
    Platform {
        id: "bmm100",
        label: "BMM Narrow",
        product: Product::Bmm100,
        viewports: BMM100_VIEWPORTS,
    },
    Platform {
        id: "bmm101",
        label: "BMM",
        product: Product::Bmm101,
        viewports: BMM101_VIEWPORTS,
    },
    Platform {
        id: "bfm100",
        label: "BFM",
        product: Product::Bfm100,
        viewports: BFM100_VIEWPORTS,
    },
];

/// The platform the host tools open when none is requested.
#[must_use]
pub fn default_platform() -> &'static Platform {
    &PLATFORMS[0]
}

/// Look up a platform by id, accepting any case
/// so `--platform BMC100` and `bmc100:small` name the same platform.
#[must_use]
pub fn platform(id: &str) -> Option<&'static Platform> {
    PLATFORMS.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// Resolve an explicit platform id, or the default when omitted.
///
/// # Errors
/// When `requested` names no platform in the catalog.
pub fn select(requested: Option<&str>) -> Result<&'static Platform, CatalogError> {
    match requested {
        None => Ok(default_platform()),
        Some(id) => platform(id).ok_or_else(|| CatalogError::platform(id)),
    }
}

/// One previewable geometry on one platform.
#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub platform: &'static Platform,
    pub viewport: &'static Viewport,
}

impl Target {
    /// # Errors
    /// When the platform id is unknown,
    /// or the viewport id is not one that platform offers.
    pub fn new(platform_id: &str, viewport_id: &str) -> Result<Self, CatalogError> {
        let platform = platform(platform_id).ok_or_else(|| CatalogError::platform(platform_id))?;
        let viewport = platform
            .viewport(viewport_id)
            .ok_or_else(|| CatalogError::viewport(platform, viewport_id))?;
        Ok(Self { platform, viewport })
    }
}

impl FromStr for Target {
    type Err = CatalogError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (platform_id, viewport_id) = s
            .split_once(':')
            .ok_or_else(|| CatalogError::malformed(s))?;
        Self::new(platform_id, viewport_id)
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.platform.id, self.viewport.id)
    }
}

/// A name that resolves to nothing in the catalog.
/// Carries the valid alternatives, so the caller can print them.
#[derive(Debug)]
pub struct CatalogError {
    message: String,
}

impl CatalogError {
    fn platform(requested: &str) -> Self {
        let available = PLATFORMS
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            message: format!("unknown platform '{requested}'; available platforms: {available}"),
        }
    }

    fn viewport(platform: &Platform, requested: &str) -> Self {
        let available = platform
            .viewports
            .iter()
            .map(|v| v.id)
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            message: format!(
                "platform '{}' has no viewport '{requested}'; available viewports: {available}",
                platform.id
            ),
        }
    }

    fn malformed(requested: &str) -> Self {
        Self {
            message: format!("target '{requested}' must be written '<platform>:<viewport>'"),
        }
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CatalogError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_lowercase_and_unique() {
        let mut platform_ids = Vec::new();
        for p in PLATFORMS {
            assert_eq!(
                p.id,
                p.id.to_ascii_lowercase(),
                "platform id must be lowercase"
            );
            assert!(
                !platform_ids.contains(&p.id),
                "duplicate platform id '{}'",
                p.id
            );
            platform_ids.push(p.id);

            let mut viewport_ids = Vec::new();
            for v in p.viewports {
                assert_eq!(
                    v.id,
                    v.id.to_ascii_lowercase(),
                    "viewport id must be lowercase"
                );
                assert!(
                    !viewport_ids.contains(&v.id),
                    "duplicate viewport id '{}' on '{}'",
                    v.id,
                    p.id
                );
                viewport_ids.push(v.id);
            }
        }
    }

    #[test]
    fn every_target_round_trips_through_its_string() {
        for p in PLATFORMS {
            for v in p.viewports {
                let target = Target {
                    platform: p,
                    viewport: v,
                };
                let parsed: Target = target
                    .to_string()
                    .parse()
                    .expect("BUG: a target's own string must parse back");
                assert_eq!(parsed.platform.id, p.id);
                assert_eq!(parsed.viewport.id, v.id);
            }
        }
    }

    #[test]
    fn fullscreen_viewports_match_the_platform_display() {
        for p in PLATFORMS {
            let display = p.display();
            for v in p.viewports {
                if matches!(v.placement, Placement::Fullscreen) {
                    assert_eq!(
                        (v.width, v.height, v.shape),
                        (display.logical_width, display.logical_height, display.shape),
                        "fullscreen viewport must match {}'s display",
                        p.id
                    );
                }
            }
        }
    }

    #[test]
    fn slot_spans_fit_their_platform_grid() {
        for p in PLATFORMS {
            for v in p.viewports {
                let Placement::SlotSpan(span) = v.placement else {
                    continue;
                };
                let grid = p
                    .slot_grid()
                    .expect("BUG: a slot-span viewport requires a slot grid");
                assert!(
                    span.columns <= grid.columns && span.rows <= grid.rows,
                    "'{}:{}' spans more slots than {} has",
                    p.id,
                    v.id,
                    p.id
                );
            }
        }
    }

    #[test]
    fn bmc100_carries_the_deck_slot_grid_and_strip() {
        let deck = platform("bmc100").expect("BUG: BMC100 must exist");
        assert_eq!(deck.label, "Deck");
        assert_eq!(deck.display().dpi, 217);
        let grid = deck.slot_grid().expect("BUG: BMC100 must have a slot grid");
        assert_eq!((grid.columns, grid.rows), (4, 2));
        assert_eq!(deck.led_count(), Some(10));

        let large = deck.viewport("large").expect("BUG: Large must exist");
        assert_eq!((large.width, large.height), (638, 480));
        assert_eq!(
            large.placement,
            Placement::SlotSpan(SlotSpan {
                columns: 2,
                rows: 2
            })
        );
    }

    #[test]
    fn stripless_platforms_have_no_grid_or_strip() {
        for id in ["bmm100", "bmm101", "bfm100"] {
            let p = platform(id).expect("BUG: platform must exist");
            assert_eq!(p.slot_grid(), None, "{id} must have no slot grid");
            assert_eq!(p.led_count(), None, "{id} must have no LED strip");
            assert_eq!(p.viewports.len(), 1, "{id} must offer one viewport");
        }
    }

    #[test]
    fn bfm100_is_round_end_to_end() {
        let bfm = platform("bfm100").expect("BUG: BFM100 must exist");
        let display = bfm.runtime_display_info();
        assert_eq!(
            (display.width, display.height, display.dpi),
            (480, 480, 229)
        );
        assert_eq!(display.shape, bmc_wasm_protocol::DisplayShape::Round);
        assert_eq!(
            bfm.viewports[0].runtime_viewport_shape(),
            bmc_wasm_protocol::ViewportShape::Round
        );
    }

    #[test]
    fn bmm101_is_rectangular_end_to_end() {
        let bmm = platform("bmm101").expect("BUG: BMM101 must exist");
        let display = bmm.runtime_display_info();
        assert_eq!(
            (display.width, display.height, display.dpi),
            (480, 320, 165)
        );
        assert_eq!(display.shape, bmc_wasm_protocol::DisplayShape::Rectangular);
        assert_eq!(
            bmm.viewports[0].runtime_viewport_shape(),
            bmc_wasm_protocol::ViewportShape::Rectangular
        );
    }

    #[test]
    fn select_defaults_to_bmc100_and_accepts_any_case() {
        assert_eq!(
            select(None).expect("BUG: default must resolve").id,
            "bmc100"
        );
        assert_eq!(
            select(Some("BMC100"))
                .expect("BUG: legacy uppercase id must resolve")
                .id,
            "bmc100"
        );
    }

    #[test]
    fn unknown_ids_list_what_is_available() {
        let err = select(Some("NOPE")).expect_err("BUG: unknown platform must fail");
        let message = err.to_string();
        for id in ["NOPE", "bmc100", "bmm100", "bmm101", "bfm100"] {
            assert!(message.contains(id), "{message}");
        }

        let err = "bmm100:small"
            .parse::<Target>()
            .expect_err("BUG: BMM100 has no Small viewport");
        let message = err.to_string();
        assert!(
            message.contains("small") && message.contains("full"),
            "{message}"
        );

        let err = "bmc100"
            .parse::<Target>()
            .expect_err("BUG: bare platform id must fail");
        assert!(err.to_string().contains("<platform>:<viewport>"), "{err}");
    }
}

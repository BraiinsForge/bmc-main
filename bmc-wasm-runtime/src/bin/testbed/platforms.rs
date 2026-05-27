// Copyright (C) 2026  Braiins Systems s.r.o.

//! Testbed platform catalog: the logical display, shape, optional slot grid,
//! optional LED strip, and previewable widget viewports per platform.
//!
//! Display shape, viewport shape, and DPI come from the testbed platform
//! catalog. The preview UI uses the viewport shape for visual treatment, and
//! the Stage 4 runtime geometry API passes the same catalog values through to
//! widgets via `widget_viewport()` and `display_info()`.

use serde::Deserialize;

/// Explicit fake DPI used by every initial platform until panel active-area
/// data is available.
pub(super) const FAKE_DPI: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisplayShape {
    Rectangular,
    Round,
}

impl DisplayShape {
    pub(super) fn to_runtime_display_shape(self) -> bmc_wasm_protocol::DisplayShape {
        match self {
            Self::Rectangular => bmc_wasm_protocol::DisplayShape::Rectangular,
            Self::Round => bmc_wasm_protocol::DisplayShape::Round,
        }
    }

    pub(super) fn to_runtime_viewport_shape(self) -> bmc_wasm_protocol::ViewportShape {
        match self {
            Self::Rectangular => bmc_wasm_protocol::ViewportShape::Rectangular,
            Self::Round => bmc_wasm_protocol::ViewportShape::Round,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DisplayProfile {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) shape: DisplayShape,
    pub(super) dpi: u32,
}

impl DisplayProfile {
    pub(super) fn to_runtime_display_info(self) -> bmc_wasm_runtime::RuntimeDisplayInfo {
        bmc_wasm_runtime::RuntimeDisplayInfo {
            width: self.width,
            height: self.height,
            shape: self.shape.to_runtime_display_shape(),
            dpi: self.dpi,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SlotGrid {
    pub(super) columns: u32,
    pub(super) rows: u32,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum LedStripKind {
    Apa102,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LedStrip {
    pub(super) kind: LedStripKind,
    pub(super) led_count: u32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SlotSpan {
    pub(super) columns: u32,
    pub(super) rows: u32,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Placement {
    Fullscreen,
    SlotSpan(SlotSpan),
}

#[derive(Debug, Clone)]
pub(super) struct WidgetViewport {
    pub(super) label: String,
    pub(super) placement: Placement,
    pub(super) shape: DisplayShape,
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(Debug, Clone)]
pub(super) struct Platform {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) display: DisplayProfile,
    pub(super) slot_grid: Option<SlotGrid>,
    pub(super) led_strip: Option<LedStrip>,
    pub(super) widget_viewports: Vec<WidgetViewport>,
}

#[derive(Debug, Clone)]
pub(super) struct PlatformCatalog {
    pub(super) default_platform: String,
    pub(super) platforms: Vec<Platform>,
}

// ── Raw serde shapes (validated into the typed model above) ──────────

#[derive(Debug, Deserialize)]
struct RawCatalog {
    default_platform: String,
    platforms: Vec<RawPlatform>,
}

#[derive(Debug, Deserialize)]
struct RawPlatform {
    id: String,
    label: String,
    display: RawDisplay,
    #[serde(default)]
    slot_grid: Option<RawSlotGrid>,
    #[serde(default)]
    led_strip: Option<RawLedStrip>,
    widget_viewports: Vec<RawViewport>,
}

#[derive(Debug, Deserialize)]
struct RawDisplay {
    width: u32,
    height: u32,
    shape: String,
    dpi: u32,
}

#[derive(Debug, Deserialize)]
struct RawSlotGrid {
    columns: u32,
    rows: u32,
}

#[derive(Debug, Deserialize)]
struct RawLedStrip {
    kind: String,
    led_count: u32,
}

#[derive(Debug, Deserialize)]
struct RawViewport {
    label: String,
    placement: RawPlacement,
    shape: String,
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
struct RawPlacement {
    #[serde(default)]
    fullscreen: Option<RawFullscreen>,
    #[serde(default)]
    slot_span: Option<RawSlotSpan>,
}

#[derive(Debug, Deserialize)]
struct RawFullscreen {}

#[derive(Debug, Deserialize)]
struct RawSlotSpan {
    columns: u32,
    rows: u32,
}

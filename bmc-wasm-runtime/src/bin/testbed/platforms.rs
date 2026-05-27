// Copyright (C) 2026  Braiins Systems s.r.o.

//! Testbed platform catalog: the logical display, shape, optional slot grid,
//! optional LED strip, and previewable widget viewports per platform.
//!
//! Display shape, viewport shape, and DPI come from the testbed platform
//! catalog. The preview UI uses the viewport shape for visual treatment, and
//! the Stage 4 runtime geometry API passes the same catalog values through to
//! widgets via `widget_viewport()` and `display_info()`.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 2 defines the catalog model before later testbed tasks consume it"
    )
)]

use std::collections::BTreeSet;

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

impl PlatformCatalog {
    /// Parse and fully validate a catalog from JSON text. Returns a single
    /// human-readable error string on the first problem found.
    pub(super) fn parse(json: &str) -> Result<Self, String> {
        let raw: RawCatalog = serde_json::from_str(json)
            .map_err(|e| format!("invalid platform catalog JSON: {e}"))?;

        if raw.platforms.is_empty() {
            return Err("platform catalog must contain at least one platform".to_owned());
        }

        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut platforms = Vec::with_capacity(raw.platforms.len());
        for rp in &raw.platforms {
            if !seen.insert(rp.id.as_str()) {
                return Err(format!("duplicate platform id '{}'", rp.id));
            }
            platforms.push(Platform::from_raw(rp)?);
        }

        if !platforms.iter().any(|p| p.id == raw.default_platform) {
            return Err(format!(
                "default_platform '{}' is not a platform id in the catalog",
                raw.default_platform
            ));
        }

        Ok(Self {
            default_platform: raw.default_platform,
            platforms,
        })
    }

    /// Look up a platform by id.
    pub(super) fn platform(&self, id: &str) -> Option<&Platform> {
        self.platforms.iter().find(|p| p.id == id)
    }
}

fn parse_shape(raw: &str) -> Result<DisplayShape, String> {
    match raw {
        "rectangular" => Ok(DisplayShape::Rectangular),
        "round" => Ok(DisplayShape::Round),
        other => Err(format!("unknown display shape '{other}'")),
    }
}

impl Platform {
    fn from_raw(raw: &RawPlatform) -> Result<Self, String> {
        let shape = parse_shape(&raw.display.shape)?;
        if raw.display.width == 0 || raw.display.height == 0 {
            return Err(format!(
                "platform '{}': display width and height must be nonzero",
                raw.id
            ));
        }
        let display = DisplayProfile {
            width: raw.display.width,
            height: raw.display.height,
            shape,
            dpi: raw.display.dpi,
        };

        let slot_grid = match raw.slot_grid.as_ref() {
            None => None,
            Some(g) => {
                if g.columns == 0 || g.rows == 0 {
                    return Err(format!(
                        "platform '{}': slot_grid columns and rows must be nonzero",
                        raw.id
                    ));
                }
                Some(SlotGrid {
                    columns: g.columns,
                    rows: g.rows,
                })
            }
        };

        let led_strip = match raw.led_strip.as_ref() {
            None => None,
            Some(l) => match l.kind.as_str() {
                "apa102" => {
                    if l.led_count == 0 {
                        return Err(format!("platform '{}': led_count must be nonzero", raw.id));
                    }
                    Some(LedStrip {
                        kind: LedStripKind::Apa102,
                        led_count: l.led_count,
                    })
                }
                other => return Err(format!("unknown led strip kind '{other}'")),
            },
        };

        if raw.widget_viewports.is_empty() {
            return Err(format!(
                "platform '{}': must declare at least one widget viewport",
                raw.id
            ));
        }

        let mut widget_viewports = Vec::with_capacity(raw.widget_viewports.len());
        for rv in &raw.widget_viewports {
            widget_viewports.push(viewport_from_raw(&raw.id, rv, slot_grid)?);
        }

        Ok(Self {
            id: raw.id.clone(),
            label: raw.label.clone(),
            display,
            slot_grid,
            led_strip,
            widget_viewports,
        })
    }
}

fn viewport_from_raw(
    platform_id: &str,
    raw: &RawViewport,
    slot_grid: Option<SlotGrid>,
) -> Result<WidgetViewport, String> {
    let shape = parse_shape(&raw.shape)?;
    if raw.width == 0 || raw.height == 0 {
        return Err(format!(
            "platform '{platform_id}': viewport width and height must be nonzero for '{}'",
            raw.label
        ));
    }

    let placement = match (&raw.placement.fullscreen, &raw.placement.slot_span) {
        (Some(_), None) => Placement::Fullscreen,
        (None, Some(s)) => {
            let grid = slot_grid.ok_or_else(|| {
                format!(
                    "platform '{platform_id}': viewport '{}': slot_span viewport requires a slot_grid",
                    raw.label
                )
            })?;
            if s.columns == 0 || s.rows == 0 {
                return Err(format!(
                    "platform '{platform_id}': viewport '{}': slot_span columns and rows must be nonzero",
                    raw.label
                ));
            }
            if s.columns > grid.columns || s.rows > grid.rows {
                return Err(format!(
                    "platform '{platform_id}': viewport '{}': slot_span columns and rows must fit within slot_grid",
                    raw.label
                ));
            }
            Placement::SlotSpan(SlotSpan {
                columns: s.columns,
                rows: s.rows,
            })
        }
        _ => {
            return Err(format!(
                "platform '{platform_id}': viewport '{}': viewport placement must be exactly one of fullscreen or slot_span",
                raw.label
            ));
        }
    };

    Ok(WidgetViewport {
        label: raw.label.clone(),
        placement,
        shape,
        width: raw.width,
        height: raw.height,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED: &str = include_str!("platforms.default.json");

    fn good_single() -> &'static str {
        r#"{
          "default_platform": "BMM100",
          "platforms": [
            {
              "id": "BMM100",
              "label": "BMM Narrow",
              "display": { "width": 160, "height": 480, "shape": "rectangular", "dpi": 1 },
              "slot_grid": null,
              "led_strip": null,
              "widget_viewports": [
                { "label": "Fullscreen", "placement": { "fullscreen": {} }, "shape": "rectangular", "width": 160, "height": 480 }
              ]
            }
          ]
        }"#
    }

    #[test]
    fn parses_a_minimal_catalog() {
        let cat = PlatformCatalog::parse(good_single()).expect("BUG: minimal catalog must parse");
        assert_eq!(cat.default_platform, "BMM100");
        assert_eq!(cat.platforms.len(), 1);
        let p = &cat.platforms[0];
        assert_eq!(p.id, "BMM100");
        assert_eq!(p.display.width, 160);
        assert_eq!(p.display.height, 480);
        assert!(matches!(p.display.shape, DisplayShape::Rectangular));
        assert!(p.slot_grid.is_none());
        assert!(p.led_strip.is_none());
        assert_eq!(p.widget_viewports.len(), 1);
        assert!(matches!(
            p.widget_viewports[0].placement,
            Placement::Fullscreen
        ));
    }

    #[test]
    fn rejects_empty_platforms() {
        let json = r#"{ "default_platform": "X", "platforms": [] }"#;
        let err = PlatformCatalog::parse(json).expect_err("BUG: empty platforms must fail");
        assert!(err.contains("at least one platform"), "{err}");
    }

    #[test]
    fn rejects_duplicate_platform_ids() {
        let json = r#"{
          "default_platform": "DUP",
          "platforms": [
            { "id": "DUP", "label": "a", "display": { "width": 160, "height": 480, "shape": "rectangular", "dpi": 1 },
              "widget_viewports": [ { "label": "F", "placement": { "fullscreen": {} }, "shape": "rectangular", "width": 160, "height": 480 } ] },
            { "id": "DUP", "label": "b", "display": { "width": 320, "height": 480, "shape": "rectangular", "dpi": 1 },
              "widget_viewports": [ { "label": "F", "placement": { "fullscreen": {} }, "shape": "rectangular", "width": 320, "height": 480 } ] }
          ]
        }"#;
        let err = PlatformCatalog::parse(json).expect_err("BUG: duplicate platform ids must fail");
        assert!(err.contains("duplicate platform id 'DUP'"), "{err}");
    }

    #[test]
    fn rejects_unknown_display_shape() {
        let json = good_single().replace(
            "\"shape\": \"rectangular\", \"dpi\"",
            "\"shape\": \"hexagon\", \"dpi\"",
        );
        let err = PlatformCatalog::parse(&json).expect_err("BUG: unknown display shape must fail");
        assert!(err.contains("unknown display shape 'hexagon'"), "{err}");
    }

    #[test]
    fn rejects_zero_display_dimensions() {
        let json = good_single().replace(
            "\"width\": 160, \"height\": 480, \"shape\": \"rectangular\"",
            "\"width\": 0, \"height\": 480, \"shape\": \"rectangular\"",
        );
        let err =
            PlatformCatalog::parse(&json).expect_err("BUG: zero display dimensions must fail");
        assert!(
            err.contains("display width and height must be nonzero"),
            "{err}"
        );
    }

    #[test]
    fn rejects_empty_widget_viewports() {
        let json = good_single().replace(
            "\"widget_viewports\": [\n                { \"label\": \"Fullscreen\", \"placement\": { \"fullscreen\": {} }, \"shape\": \"rectangular\", \"width\": 160, \"height\": 480 }\n              ]",
            "\"widget_viewports\": []",
        );
        let err = PlatformCatalog::parse(&json).expect_err("BUG: empty widget viewports must fail");
        assert!(err.contains("at least one widget viewport"), "{err}");
    }

    #[test]
    fn rejects_unknown_viewport_placement() {
        let json = good_single().replace("{ \"fullscreen\": {} }", "{}");
        let err =
            PlatformCatalog::parse(&json).expect_err("BUG: unknown viewport placement must fail");
        assert!(
            err.contains("viewport placement must be exactly one of fullscreen or slot_span"),
            "{err}"
        );
    }

    #[test]
    fn rejects_slot_span_without_slot_grid() {
        let json = good_single().replace(
            "{ \"fullscreen\": {} }",
            "{ \"slot_span\": { \"columns\": 2, \"rows\": 2 } }",
        );
        let err =
            PlatformCatalog::parse(&json).expect_err("BUG: slot_span without slot_grid must fail");
        assert!(
            err.contains("slot_span viewport requires a slot_grid"),
            "{err}"
        );
    }

    #[test]
    fn rejects_zero_slot_grid_dimensions() {
        let json = good_single().replace(
            "\"slot_grid\": null",
            "\"slot_grid\": { \"columns\": 0, \"rows\": 2 }",
        );
        let err =
            PlatformCatalog::parse(&json).expect_err("BUG: zero slot_grid dimensions must fail");
        assert!(
            err.contains("slot_grid columns and rows must be nonzero"),
            "{err}"
        );
    }

    #[test]
    fn rejects_zero_slot_span_dimensions() {
        let json = good_single()
            .replace(
                "\"slot_grid\": null",
                "\"slot_grid\": { \"columns\": 2, \"rows\": 2 }",
            )
            .replace(
                "{ \"fullscreen\": {} }",
                "{ \"slot_span\": { \"columns\": 0, \"rows\": 1 } }",
            );
        let err =
            PlatformCatalog::parse(&json).expect_err("BUG: zero slot_span dimensions must fail");
        assert!(
            err.contains("slot_span columns and rows must be nonzero"),
            "{err}"
        );
    }

    #[test]
    fn rejects_slot_span_larger_than_slot_grid() {
        let json = good_single()
            .replace(
                "\"slot_grid\": null",
                "\"slot_grid\": { \"columns\": 4, \"rows\": 2 }",
            )
            .replace(
                "{ \"fullscreen\": {} }",
                "{ \"slot_span\": { \"columns\": 5, \"rows\": 1 } }",
            );
        let err = PlatformCatalog::parse(&json)
            .expect_err("BUG: oversized slot_span dimensions must fail");
        assert!(
            err.contains("slot_span columns and rows must fit within slot_grid"),
            "{err}"
        );
    }

    #[test]
    fn rejects_zero_viewport_dimensions() {
        let json = good_single().replace(
            "\"shape\": \"rectangular\", \"width\": 160, \"height\": 480 }\n              ]",
            "\"shape\": \"rectangular\", \"width\": 0, \"height\": 480 }\n              ]",
        );
        let err =
            PlatformCatalog::parse(&json).expect_err("BUG: zero viewport dimensions must fail");
        assert!(
            err.contains("viewport width and height must be nonzero"),
            "{err}"
        );
    }

    #[test]
    fn rejects_unknown_led_strip_kind() {
        let json = good_single().replace(
            "\"led_strip\": null",
            "\"led_strip\": { \"kind\": \"ws2812\", \"led_count\": 10 }",
        );
        let err = PlatformCatalog::parse(&json).expect_err("BUG: unknown LED strip kind must fail");
        assert!(err.contains("unknown led strip kind 'ws2812'"), "{err}");
    }

    #[test]
    fn rejects_zero_led_count() {
        let json = good_single().replace(
            "\"led_strip\": null",
            "\"led_strip\": { \"kind\": \"apa102\", \"led_count\": 0 }",
        );
        let err = PlatformCatalog::parse(&json).expect_err("BUG: zero led_count must fail");
        assert!(err.contains("led_count must be nonzero"), "{err}");
    }

    #[test]
    fn rejects_default_platform_not_in_table() {
        let json = good_single().replace(
            "\"default_platform\": \"BMM100\"",
            "\"default_platform\": \"NOPE\"",
        );
        let err =
            PlatformCatalog::parse(&json).expect_err("BUG: unknown default_platform must fail");
        assert!(
            err.contains("default_platform 'NOPE' is not a platform id"),
            "{err}"
        );
    }

    #[test]
    fn bundled_catalog_parses() {
        let cat = PlatformCatalog::parse(BUNDLED).expect("BUG: bundled catalog must parse");
        let ids: Vec<&str> = cat.platforms.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["BMC100", "BMM100", "BMM101", "BFM100"]);

        let deck = cat.platform("BMC100").expect("BUG: BMC100 must exist");
        assert_eq!(deck.label, "Deck");
        assert_eq!(deck.display.dpi, FAKE_DPI);
        let slot_grid = deck.slot_grid.expect("BUG: BMC100 must have slot grid");
        assert_eq!((slot_grid.columns, slot_grid.rows), (4, 2));
        let led_strip = deck.led_strip.expect("BUG: BMC100 must have LED strip");
        assert!(matches!(led_strip.kind, LedStripKind::Apa102));
        assert_eq!(led_strip.led_count, 10);

        let large = &deck.widget_viewports[1];
        assert_eq!(large.label, "Large");
        assert!(matches!(large.shape, DisplayShape::Rectangular));
        assert_eq!((large.width, large.height), (638, 480));
        let Placement::SlotSpan(span) = large.placement else {
            panic!("BUG: large viewport must use slot_span");
        };
        assert_eq!((span.columns, span.rows), (2, 2));
    }

    #[test]
    fn bfm100_converts_to_round_runtime_display_info() {
        let cat = PlatformCatalog::parse(BUNDLED).expect("BUG: bundled catalog must parse");
        let p = cat.platform("BFM100").expect("BUG: BFM100 must exist");
        let display = p.display.to_runtime_display_info();

        assert_eq!(
            (display.width, display.height, display.dpi),
            (480, 480, FAKE_DPI)
        );
        assert_eq!(display.shape, bmc_wasm_protocol::DisplayShape::Round);
    }

    #[test]
    fn bmm101_converts_to_rectangular_runtime_display_info() {
        let cat = PlatformCatalog::parse(BUNDLED).expect("BUG: bundled catalog must parse");
        let p = cat.platform("BMM101").expect("BUG: BMM101 must exist");
        let display = p.display.to_runtime_display_info();

        assert_eq!(
            (display.width, display.height, display.dpi),
            (480, 320, FAKE_DPI)
        );
        assert_eq!(display.shape, bmc_wasm_protocol::DisplayShape::Rectangular);
    }

    #[test]
    fn catalog_shape_converts_to_viewport_shape() {
        assert_eq!(
            DisplayShape::Rectangular.to_runtime_viewport_shape(),
            bmc_wasm_protocol::ViewportShape::Rectangular
        );
        assert_eq!(
            DisplayShape::Round.to_runtime_viewport_shape(),
            bmc_wasm_protocol::ViewportShape::Round
        );
    }
}

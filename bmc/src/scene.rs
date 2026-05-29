// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_widget_manifest::{ParamKey, ParamValue};
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::time::Duration;
use uuid::Uuid;

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WidgetSize {
    Small,
    Medium,
    Large,
    Full,
}

impl Display for WidgetSize {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Full => "full",
            Self::Large => "large",
            Self::Medium => "medium",
            Self::Small => "small",
        };
        f.write_str(value)
    }
}

/// Where a widget sits on the display: full-screen, or spanning a block of
/// grid slots.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WidgetPlacement {
    /// Occupies the entire display.
    Fullscreen,
    /// Occupies a rectangular block of grid slots.
    SlotSpan(SlotSpan),
}

/// A rectangular span of grid slots.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SlotSpan {
    /// Number of columns spanned.
    pub columns: u32,
    /// Number of rows spanned.
    pub rows: u32,
}

impl WidgetPlacement {
    #[must_use]
    pub fn row_span(&self) -> u8 {
        match self {
            Self::Fullscreen => u8::try_from(WidgetPosition::MAX_ROWS).unwrap_or(u8::MAX),
            Self::SlotSpan(s) => u8::try_from(s.rows).unwrap_or(u8::MAX),
        }
    }

    #[must_use]
    pub fn col_span(&self) -> u8 {
        match self {
            Self::Fullscreen => u8::try_from(WidgetPosition::MAX_COLS).unwrap_or(u8::MAX),
            Self::SlotSpan(s) => u8::try_from(s.columns).unwrap_or(u8::MAX),
        }
    }
}

impl From<WidgetSize> for WidgetPlacement {
    fn from(size: WidgetSize) -> Self {
        match size {
            WidgetSize::Small => Self::SlotSpan(SlotSpan {
                columns: 1,
                rows: 1,
            }),
            WidgetSize::Medium => Self::SlotSpan(SlotSpan {
                columns: 2,
                rows: 1,
            }),
            WidgetSize::Large => Self::SlotSpan(SlotSpan {
                columns: 2,
                rows: 2,
            }),
            WidgetSize::Full => Self::Fullscreen,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct WidgetPosition {
    pub row: u8,
    pub col: u8,
}

impl WidgetPosition {
    pub const MAX_ROWS: usize = 2;
    pub const MAX_COLS: usize = 4;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct WidgetId(Uuid);

impl WidgetId {
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Display for WidgetId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for WidgetId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Widget {
    pub id: WidgetId,
    #[serde(flatten)]
    pub position: WidgetPosition,
    pub placement: WidgetPlacement,
    pub widget_type_id: Uuid,
    #[serde(default = "default_widget_viewport_shape")]
    pub viewport_shape: bmc_widget_manifest::ViewportShape,
    #[serde(default)]
    pub params: BTreeMap<ParamKey, ParamValue>,
}

fn default_widget_viewport_shape() -> bmc_widget_manifest::ViewportShape {
    bmc_widget_manifest::ViewportShape::Rectangular
}

#[derive(Deserialize)]
struct RawWidget {
    id: WidgetId,
    #[serde(flatten)]
    position: WidgetPosition,
    #[serde(default)]
    placement: Option<WidgetPlacement>,
    #[serde(default)]
    size: Option<WidgetSize>,
    widget_type_id: Uuid,
    #[serde(default = "default_widget_viewport_shape")]
    viewport_shape: bmc_widget_manifest::ViewportShape,
    #[serde(default)]
    params: BTreeMap<ParamKey, ParamValue>,
}

impl<'de> Deserialize<'de> for Widget {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = RawWidget::deserialize(d)?;
        let placement = match (raw.placement, raw.size) {
            (Some(p), _) => p,
            (None, Some(size)) => WidgetPlacement::from(size),
            (None, None) => {
                return Err(serde::de::Error::custom(
                    "widget requires `placement` or legacy `size`",
                ));
            }
        };
        Ok(Self {
            id: raw.id,
            position: raw.position,
            placement,
            viewport_shape: raw.viewport_shape,
            widget_type_id: raw.widget_type_id,
            params: raw.params,
        })
    }
}

impl Widget {
    #[must_use]
    pub fn new(
        widget_type_id: Uuid,
        params: BTreeMap<ParamKey, ParamValue>,
        position: WidgetPosition,
        placement: WidgetPlacement,
    ) -> Self {
        Self {
            id: WidgetId::generate(),
            widget_type_id,
            viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
            params,
            position,
            placement,
        }
    }

    #[must_use]
    pub fn clone_with_new_id(&self) -> Self {
        let mut cloned = self.clone();
        cloned.id = WidgetId::generate();
        cloned
    }

    #[must_use]
    pub fn in_bounds(&self) -> bool {
        let bottom = usize::from(self.position.row) + usize::from(self.placement.row_span());
        let right = usize::from(self.position.col) + usize::from(self.placement.col_span());
        (bottom <= WidgetPosition::MAX_ROWS) && (right <= WidgetPosition::MAX_COLS)
    }

    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        let self_left = usize::from(self.position.col);
        let self_right = self_left + usize::from(self.placement.col_span());
        let self_top = usize::from(self.position.row);
        let self_bottom = self_top + usize::from(self.placement.row_span());

        let other_left = usize::from(other.position.col);
        let other_right = other_left + usize::from(other.placement.col_span());
        let other_top = usize::from(other.position.row);
        let other_bottom = other_top + usize::from(other.placement.row_span());

        (self_left < other_right)
            && (self_right > other_left)
            && (self_top < other_bottom)
            && (self_bottom > other_top)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SceneKind {
    Fullscreen,
    Combined,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct SceneId(Uuid);

impl SceneId {
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Display for SceneId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for SceneId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub id: SceneId,
    pub enabled: bool,
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub cycle_duration: Option<Duration>,
    pub kind: SceneKind,
    #[serde(
        serialize_with = "serialize_widgets",
        deserialize_with = "deserialize_widgets"
    )]
    pub widgets: IndexMap<WidgetId, Widget>,
}

impl Scene {
    pub const MIN_CYCLE_DURATION: Duration = Duration::from_secs(1);

    #[must_use]
    pub fn fullscreen(widget_uid: Uuid, params: BTreeMap<ParamKey, ParamValue>) -> Self {
        let widget = Widget::new(
            widget_uid,
            params,
            WidgetPosition { row: 0, col: 0 },
            WidgetPlacement::Fullscreen,
        );
        Self {
            id: SceneId::generate(),
            enabled: true,
            cycle_duration: None,
            kind: SceneKind::Fullscreen,
            widgets: indexmap::indexmap! {
                widget.id => widget
            },
        }
    }

    #[must_use]
    pub fn combined() -> Self {
        Self {
            id: SceneId::generate(),
            enabled: true,
            cycle_duration: None,
            kind: SceneKind::Combined,
            widgets: IndexMap::with_capacity(0),
        }
    }

    #[must_use]
    pub fn clone_with_new_id(&self) -> Self {
        let mut cloned = self.clone();
        cloned.id = SceneId::generate();
        cloned.widgets = cloned
            .widgets
            .into_values()
            .map(|widget| {
                let widget = widget.clone_with_new_id();
                (widget.id, widget)
            })
            .collect();
        cloned
    }
}

#[inline]
pub fn serialize_scenes<S: Serializer>(
    map: &IndexMap<SceneId, Scene>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(map.values())
}

#[inline]
pub fn deserialize_scenes<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<IndexMap<SceneId, Scene>, D::Error> {
    let vec = Vec::<Scene>::deserialize(deserializer)?;
    let map = vec.into_iter().map(|scene| (scene.id, scene)).collect();
    Ok(map)
}

#[inline]
fn serialize_widgets<S: Serializer>(
    map: &IndexMap<WidgetId, Widget>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(map.values())
}

#[inline]
fn deserialize_widgets<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<IndexMap<WidgetId, Widget>, D::Error> {
    let vec = Vec::<Widget>::deserialize(deserializer)?;
    let map = vec.into_iter().map(|widget| (widget.id, widget)).collect();
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_bounds_does_not_overflow_u8_addition() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition {
                row: u8::MAX,
                col: 0,
            },
            placement: WidgetPlacement::SlotSpan(SlotSpan {
                columns: 1,
                rows: 1,
            }),
            widget_type_id: Uuid::nil(),
            viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
            params: BTreeMap::new(),
        };
        assert!(
            !widget.in_bounds(),
            "widget at row=255 must not be in bounds"
        );
    }

    #[test]
    fn overlaps_does_not_overflow_u8_addition() {
        let a = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition {
                row: u8::MAX,
                col: 0,
            },
            placement: WidgetPlacement::SlotSpan(SlotSpan {
                columns: 1,
                rows: 1,
            }),
            widget_type_id: Uuid::nil(),
            viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
            params: BTreeMap::new(),
        };
        let b = a.clone_with_new_id();
        assert!(
            a.overlaps(&b),
            "two identical widgets at row=255 must be reported as overlapping",
        );
    }

    #[test]
    fn clone_with_new_id_preserves_widget_iteration_order() {
        let positions = [
            (
                0_u8,
                0_u8,
                WidgetPlacement::SlotSpan(SlotSpan {
                    columns: 1,
                    rows: 1,
                }),
            ),
            (
                0,
                1,
                WidgetPlacement::SlotSpan(SlotSpan {
                    columns: 1,
                    rows: 1,
                }),
            ),
            (
                0,
                2,
                WidgetPlacement::SlotSpan(SlotSpan {
                    columns: 2,
                    rows: 1,
                }),
            ),
            (
                1,
                0,
                WidgetPlacement::SlotSpan(SlotSpan {
                    columns: 2,
                    rows: 2,
                }),
            ),
        ];
        let widget_type = Uuid::new_v4();
        let mut source = Scene::combined();
        for (row, col, placement) in positions {
            let w = Widget::new(
                widget_type,
                BTreeMap::new(),
                WidgetPosition { row, col },
                placement,
            );
            source.widgets.insert(w.id, w);
        }
        let source_order: Vec<(u8, u8, WidgetPlacement)> = source
            .widgets
            .values()
            .map(|w| (w.position.row, w.position.col, w.placement.clone()))
            .collect();

        let cloned = source.clone_with_new_id();

        let cloned_order: Vec<(u8, u8, WidgetPlacement)> = cloned
            .widgets
            .values()
            .map(|w| (w.position.row, w.position.col, w.placement.clone()))
            .collect();

        assert_eq!(
            source_order, cloned_order,
            "BUG: clone_with_new_id must preserve widget iteration order"
        );
        assert_ne!(source.id, cloned.id);
        let source_widget_ids: Vec<_> = source.widgets.keys().copied().collect();
        let cloned_widget_ids: Vec<_> = cloned.widgets.keys().copied().collect();
        assert!(
            source_widget_ids
                .iter()
                .zip(cloned_widget_ids.iter())
                .all(|(a, b)| a != b),
            "BUG: each cloned widget must have a fresh id",
        );
    }

    #[test]
    fn legacy_size_migrates_to_placement_on_deserialize() {
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440010",
            "row": 0,
            "col": 0,
            "size": "small",
            "widget_type_id": "550e8400-e29b-41d4-a716-446655440011"
        }"#;
        let w: Widget = serde_json::from_str(json).expect("BUG: legacy widget must migrate");
        assert_eq!(
            w.placement,
            WidgetPlacement::SlotSpan(SlotSpan {
                columns: 1,
                rows: 1
            })
        );
    }

    #[test]
    fn legacy_full_size_migrates_to_fullscreen() {
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440010",
            "row": 0,
            "col": 0,
            "size": "full",
            "widget_type_id": "550e8400-e29b-41d4-a716-446655440011"
        }"#;
        let w: Widget = serde_json::from_str(json).expect("BUG: legacy full must migrate");
        assert_eq!(w.placement, WidgetPlacement::Fullscreen);
    }

    #[test]
    fn legacy_medium_and_large_map_to_spans() {
        let mk = |size: &str| -> Widget {
            let json = format!(
                r#"{{"id":"550e8400-e29b-41d4-a716-446655440010","row":0,"col":0,
                     "size":"{size}",
                     "widget_type_id":"550e8400-e29b-41d4-a716-446655440011"}}"#
            );
            serde_json::from_str(&json).expect("BUG: legacy size must migrate")
        };
        assert_eq!(
            mk("medium").placement,
            WidgetPlacement::SlotSpan(SlotSpan {
                columns: 2,
                rows: 1
            })
        );
        assert_eq!(
            mk("large").placement,
            WidgetPlacement::SlotSpan(SlotSpan {
                columns: 2,
                rows: 2
            })
        );
    }

    #[test]
    fn placement_serializes_in_new_shape() {
        let w = Widget::new(
            Uuid::nil(),
            BTreeMap::new(),
            WidgetPosition { row: 0, col: 0 },
            WidgetPlacement::SlotSpan(SlotSpan {
                columns: 2,
                rows: 2,
            }),
        );
        let json = serde_json::to_value(&w).expect("BUG: serialize widget");
        assert_eq!(json["placement"]["slot_span"]["columns"], 2);
        assert_eq!(json["placement"]["slot_span"]["rows"], 2);
    }

    #[test]
    fn slot_span_in_bounds_and_overlap() {
        let a = Widget::new(
            Uuid::nil(),
            BTreeMap::new(),
            WidgetPosition { row: 0, col: 0 },
            WidgetPlacement::SlotSpan(SlotSpan {
                columns: 2,
                rows: 2,
            }),
        );
        assert!(a.in_bounds());
        let b = Widget::new(
            Uuid::nil(),
            BTreeMap::new(),
            WidgetPosition { row: 0, col: 1 },
            WidgetPlacement::SlotSpan(SlotSpan {
                columns: 2,
                rows: 1,
            }),
        );
        assert!(a.overlaps(&b));
    }

    #[test]
    fn legacy_widget_config_deserializes_with_rectangular_viewport_shape() {
        let json = r#"{ "id": "00000000-0000-0000-0000-000000000000",
                        "widget_type_id": "00000000-0000-0000-0000-000000000000",
                        "row": 0, "col": 0,
                        "placement": "fullscreen" }"#;
        let widget: Widget = serde_json::from_str(json).expect("BUG: parse");
        assert_eq!(
            widget.viewport_shape,
            bmc_widget_manifest::ViewportShape::Rectangular
        );
    }
}

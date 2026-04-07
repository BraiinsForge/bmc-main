// Copyright (C) 2025  Braiins Systems s.r.o.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Display, Formatter};
use std::time::Duration;
use uuid::Uuid;

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSize {
    Small,
    Medium,
    Large,
    Full,
}

impl WidgetSize {
    #[must_use]
    pub fn row_span(&self) -> u8 {
        match self {
            Self::Small | Self::Medium => 1,
            Self::Large | Self::Full => 2,
        }
    }

    #[must_use]
    pub fn col_span(&self) -> u8 {
        match self {
            Self::Small => 1,
            Self::Medium | Self::Large => 2,
            Self::Full => 4,
        }
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        match self {
            Self::Small => 317,
            Self::Medium | Self::Large => 638,
            Self::Full => 1280,
        }
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        match self {
            Self::Small | Self::Medium => 238,
            Self::Large | Self::Full => 480,
        }
    }
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Widget {
    pub id: WidgetId,
    #[serde(flatten)]
    pub position: WidgetPosition,
    pub size: WidgetSize,
    pub widget_type_id: Uuid,
    pub params: serde_json::Value,
}

impl Widget {
    #[must_use]
    pub fn new(
        widget_type_id: Uuid,
        params: serde_json::Value,
        position: WidgetPosition,
        size: WidgetSize,
    ) -> Self {
        Self {
            id: WidgetId::generate(),
            widget_type_id,
            params,
            position,
            size,
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
        let bottom = usize::from(self.position.row) + usize::from(self.size.row_span());
        let right = usize::from(self.position.col) + usize::from(self.size.col_span());
        (bottom <= WidgetPosition::MAX_ROWS) && (right <= WidgetPosition::MAX_COLS)
    }

    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        let self_left = usize::from(self.position.col);
        let self_right = self_left + usize::from(self.size.col_span());
        let self_top = usize::from(self.position.row);
        let self_bottom = self_top + usize::from(self.size.row_span());

        let other_left = usize::from(other.position.col);
        let other_right = other_left + usize::from(other.size.col_span());
        let other_top = usize::from(other.position.row);
        let other_bottom = other_top + usize::from(other.size.row_span());

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
    pub fn fullscreen(widget_uid: Uuid, params: serde_json::Value) -> Self {
        let widget = Widget::new(
            widget_uid,
            params,
            WidgetPosition { row: 0, col: 0 },
            WidgetSize::Full,
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
            size: WidgetSize::Small,
            widget_type_id: Uuid::nil(),
            params: serde_json::Value::Null,
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
            size: WidgetSize::Small,
            widget_type_id: Uuid::nil(),
            params: serde_json::Value::Null,
        };
        let b = a.clone_with_new_id();
        assert!(
            a.overlaps(&b),
            "two identical widgets at row=255 must be reported as overlapping",
        );
    }
}

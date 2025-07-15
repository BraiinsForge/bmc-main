// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated;
use crate::indexmap_model::IndexMapModel;
use anyhow::anyhow;
use bmc_shared_time::time::Timezone;
use indexmap::{IndexMap, indexmap};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use slint::{ModelRc, ToSharedString};
use std::fmt::{Display, Formatter};
use std::hash::Hash;
use std::mem;
use std::str::FromStr;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontStyle {
    Light,
    Medium,
    Bold,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockStyle {
    AnalogRound,
    AnalogRect,
    Digital,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockWidget {
    pub clock_style: ClockStyle,
    pub numbers_font_style: FontStyle,
    pub show_date: bool,
    pub show_seconds: bool,
    pub show_timezone: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<Timezone>,
}

impl Default for ClockWidget {
    fn default() -> Self {
        Self {
            clock_style: ClockStyle::Digital,
            numbers_font_style: FontStyle::Medium,
            show_date: true,
            show_seconds: true,
            show_timezone: true,
            timezone: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "params")]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    Clock(ClockWidget),
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
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
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct WidgetPosition {
    pub row: u8,
    pub col: u8,
}

impl WidgetPosition {
    pub const MAX_ROWS: u32 = 2;
    pub const MAX_COLS: u32 = 4;
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct WidgetId(String);

impl WidgetId {
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Display for WidgetId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for WidgetId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Err(anyhow!("Empty string"))
        } else {
            Ok(Self(value.to_owned()))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Widget {
    pub id: WidgetId,
    #[serde(flatten)]
    pub position: WidgetPosition,
    pub size: WidgetSize,
    #[serde(flatten)]
    pub kind: WidgetKind,
}

impl Widget {
    #[must_use]
    pub fn clone_with_new_id(&self) -> Self {
        let mut cloned = self.clone();
        cloned.id = WidgetId::generate();

        cloned
    }

    fn overlaps(&self, other: &Self) -> bool {
        let self_left = self.position.col;
        let self_right = self_left + self.size.col_span();
        let self_top = self.position.row;
        let self_bottom = self_top + self.size.row_span();

        let other_left = other.position.col;
        let other_right = other_left + other.size.col_span();
        let other_top = other.position.row;
        let other_bottom = other_top + other.size.row_span();

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

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct SceneId(String);

impl SceneId {
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Display for SceneId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for SceneId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Err(anyhow!("Empty string"))
        } else {
            Ok(Self(value.to_owned()))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub id: SceneId,
    pub enabled: bool,
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    pub kind: SceneKind,
    #[serde(
        serialize_with = "serialize_widgets",
        deserialize_with = "deserialize_widgets"
    )]
    pub widgets: IndexMap<WidgetId, Widget>,
}

impl Scene {
    pub const MIN_DURATION: Duration = Duration::from_secs(1);
    const DEFAULT_DURATION: Duration = Duration::from_secs(5);
    const DEFAULT_ENABLED: bool = true;

    #[must_use]
    pub fn fullscreen(widget_kind: WidgetKind) -> Self {
        Self {
            id: SceneId::generate(),
            enabled: Self::DEFAULT_ENABLED,
            duration: Self::DEFAULT_DURATION,
            kind: SceneKind::Fullscreen,
            widgets: {
                let id = WidgetId::generate();

                indexmap! {
                    id.clone() => Widget {
                        id,
                        position: WidgetPosition { row: 0, col: 0 },
                        size: WidgetSize::Full,
                        kind: widget_kind,
                    }
                }
            },
        }
    }

    #[must_use]
    pub fn combined() -> Self {
        Self {
            id: SceneId::generate(),
            enabled: Self::DEFAULT_ENABLED,
            duration: Self::DEFAULT_DURATION,
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
                (widget.id.clone(), widget)
            })
            .collect();

        cloned
    }

    pub fn add_widget(
        &mut self,
        position: WidgetPosition,
        size: WidgetSize,
        kind: WidgetKind,
    ) -> Result<Widget, AddWidgetError> {
        match &self.kind {
            SceneKind::Fullscreen => {
                return Err(AddWidgetError::CannotAddWidgetToFullscreenScene);
            }
            SceneKind::Combined => {
                if size == WidgetSize::Full {
                    return Err(AddWidgetError::CannotAddFullscreenWidgetToCombinedScene);
                }
            }
        }

        let new_widget = Widget {
            id: WidgetId::generate(),
            position,
            size,
            kind,
        };

        Self::validate_widget_placement(&new_widget, self.widgets.values())
            .map_err(AddWidgetError::InvalidWidgetPlacement)?;

        let replaced_widget = self
            .widgets
            .insert(new_widget.id.clone(), new_widget.clone());
        debug_assert!(replaced_widget.is_none());

        Ok(new_widget)
    }

    pub fn update_widget(
        &mut self,
        id: &WidgetId,
        position: WidgetPosition,
        size: WidgetSize,
        kind: WidgetKind,
    ) -> Result<Widget, UpdateWidgetError> {
        let mut widget = self
            .widgets
            .get(id)
            .ok_or(UpdateWidgetError::NotFound)?
            .clone();

        if mem::discriminant(&widget.kind) != mem::discriminant(&kind) {
            return Err(UpdateWidgetError::CannotSwitchWidgetKind);
        }

        match &self.kind {
            SceneKind::Fullscreen => {
                if widget.position != position {
                    return Err(UpdateWidgetError::CannotUpdateWidgetPositionInFullscreenScene);
                }

                if widget.size != size {
                    return Err(UpdateWidgetError::CannotUpdateWidgetSizeInFullscreenScene);
                }

                widget.kind = kind;
            }
            SceneKind::Combined => {
                if size == WidgetSize::Full {
                    return Err(UpdateWidgetError::CannotUpdateWidgetSizeToFullInCombinedScene);
                }

                widget.position = position;
                widget.size = size;
                widget.kind = kind;

                Self::validate_widget_placement(
                    &widget,
                    self.widgets.values().filter(|w| w.id != widget.id),
                )
                .map_err(UpdateWidgetError::InvalidWidgetPlacement)?;
            }
        }

        let replaced_widget = self.widgets.insert(widget.id.clone(), widget.clone());
        debug_assert!(replaced_widget.is_some());

        Ok(widget)
    }

    pub fn remove_widget(&mut self, id: &WidgetId) -> Result<(), RemoveWidgetError> {
        if self.kind == SceneKind::Fullscreen {
            return Err(RemoveWidgetError::CannotRemoveWidgetFromFullscreenScene);
        }

        self.widgets
            .shift_remove(id)
            .ok_or(RemoveWidgetError::NotFound)?;

        Ok(())
    }

    fn validate_widget_placement<'a>(
        widget: &'a Widget,
        other_widgets: impl IntoIterator<Item = &'a Widget>,
    ) -> Result<(), InvalidWidgetPlacementError> {
        let bottom: u32 = (widget.position.row + widget.size.row_span()).into();
        if bottom > WidgetPosition::MAX_ROWS {
            return Err(InvalidWidgetPlacementError::OutOfBounds);
        }

        let right: u32 = (widget.position.col + widget.size.col_span()).into();
        if right > WidgetPosition::MAX_COLS {
            return Err(InvalidWidgetPlacementError::OutOfBounds);
        }

        for other_widget in other_widgets {
            if widget.overlaps(other_widget) {
                return Err(InvalidWidgetPlacementError::Overlap);
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Error)]
pub enum AddWidgetError {
    #[error("Cannot add any widget to a fullscreen scene")]
    CannotAddWidgetToFullscreenScene,
    #[error("Cannot add fullscreen widget to a combined scene")]
    CannotAddFullscreenWidgetToCombinedScene,
    #[error(transparent)]
    InvalidWidgetPlacement(#[from] InvalidWidgetPlacementError),
}

#[derive(Debug, Clone, Error)]
pub enum UpdateWidgetError {
    #[error("Widget not found")]
    NotFound,
    #[error("Cannot update widget position in a fullscreen scene")]
    CannotUpdateWidgetPositionInFullscreenScene,
    #[error("Cannot update widget size in a fullscreen scene")]
    CannotUpdateWidgetSizeInFullscreenScene,
    #[error("Cannot update widget size to full in a combined scene")]
    CannotUpdateWidgetSizeToFullInCombinedScene,
    #[error("Cannot switch widget kind")]
    CannotSwitchWidgetKind,
    #[error(transparent)]
    InvalidWidgetPlacement(#[from] InvalidWidgetPlacementError),
}

#[derive(Debug, Clone, Error)]
pub enum RemoveWidgetError {
    #[error("Widget not found")]
    NotFound,
    #[error("Cannot remove widget from a fullscreen scene")]
    CannotRemoveWidgetFromFullscreenScene,
}

#[derive(Debug, Clone, Error)]
pub enum InvalidWidgetPlacementError {
    #[error("Widget is out of bounds")]
    OutOfBounds,
    #[error("Widget overlaps with other widget")]
    Overlap,
}

#[derive(Debug)]
pub enum Screen {
    Void,
    DownloadFirmware,
    Upgrade,
    UpgradeFailed,
    UpgradeSuccess,
    InitialSetupStart,
    InitialSetupWifiConnecting,
    InitialSetupWifiConnected,
    InitialSetupWifiError,
    InitialSetupGeneralError,
    InitialSetupConnectInfo,
    InitialSetupCompleted,
    ConnectInfo,
}

impl From<Screen> for generated::UIScreen {
    fn from(value: Screen) -> Self {
        match value {
            Screen::Void => Self::Void,
            Screen::DownloadFirmware => Self::UpgradeDownload,
            Screen::Upgrade => Self::UpgradeProgress,
            Screen::UpgradeFailed => Self::UpgradeFailed,
            Screen::UpgradeSuccess => Self::UpgradeSuccess,
            Screen::InitialSetupStart => Self::InitStartConnect,
            Screen::InitialSetupWifiConnecting => Self::InitWifiConnectProgress,
            Screen::InitialSetupWifiConnected => Self::InitWifiConnectSuccess,
            Screen::InitialSetupWifiError => Self::InitWifiConnectFailed,
            Screen::InitialSetupGeneralError => Self::InitGeneralError,
            Screen::InitialSetupConnectInfo => Self::InitDeviceSetupQr,
            Screen::InitialSetupCompleted => Self::InitSetupSuccess,
            Screen::ConnectInfo => Self::ConnectInfo,
        }
    }
}

impl From<WidgetSize> for generated::WidgetSize {
    fn from(value: WidgetSize) -> Self {
        match value {
            WidgetSize::Small => Self::Small,
            WidgetSize::Medium => Self::Medium,
            WidgetSize::Large => Self::Large,
            WidgetSize::Full => Self::Full,
        }
    }
}

impl From<Widget> for generated::Widget {
    fn from(widget: Widget) -> Self {
        let mut slint_widget = generated::Widget {
            id: widget.id.to_shared_string(),
            row: widget.position.row.into(),
            col: widget.position.col.into(),
            size: widget.size.into(),
            ..generated::Widget::default()
        };

        match widget.kind {
            WidgetKind::Clock(config) => {
                slint_widget.kind = generated::WidgetKind::Clock;
                slint_widget.clock = generated::WidgetClockData {
                    config: config.into(),
                    ..generated::WidgetClockData::default()
                }
            }
        }

        slint_widget
    }
}

impl From<ClockWidget> for generated::WidgetClockConfig {
    fn from(from: ClockWidget) -> Self {
        Self {
            clock_style: match from.clock_style {
                ClockStyle::AnalogRound => generated::ClockStyle::AnalogRound,
                ClockStyle::AnalogRect => generated::ClockStyle::AnalogRect,
                ClockStyle::Digital => generated::ClockStyle::Digital,
            },
            numbers_font_style: match from.numbers_font_style {
                FontStyle::Light => generated::FontStyle::Light,
                FontStyle::Medium => generated::FontStyle::Medium,
                FontStyle::Bold => generated::FontStyle::Bold,
            },
            show_date: from.show_date,
            show_seconds: from.show_seconds,
            show_timezone: from.show_timezone,
        }
    }
}

impl From<Scene> for generated::Scene {
    fn from(value: Scene) -> Self {
        #[expect(clippy::cast_possible_truncation)]
        let duration = value.duration.as_millis() as i64;

        let widgets = value
            .widgets
            .into_iter()
            .map(|(id, widget)| (id, widget.into()))
            .collect::<IndexMapModel<_, _>>();

        Self {
            id: value.id.to_shared_string(),
            enabled: value.enabled,
            duration,
            widgets: ModelRc::new(widgets),
        }
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
    de_indexmap(deserializer, |scene: &Scene| scene.id.clone())
}

#[inline]
pub fn serialize_widgets<S: Serializer>(
    map: &IndexMap<WidgetId, Widget>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(map.values())
}

#[inline]
pub fn deserialize_widgets<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<IndexMap<WidgetId, Widget>, D::Error> {
    de_indexmap(deserializer, |widget: &Widget| widget.id.clone())
}

fn de_indexmap<'de, D: Deserializer<'de>, K: Hash + Eq, V: Deserialize<'de>>(
    deserializer: D,
    key_selector: impl Fn(&V) -> K,
) -> Result<IndexMap<K, V>, D::Error> {
    let vec = Vec::<V>::deserialize(deserializer)?;
    let map = vec
        .into_iter()
        .map(|value| (key_selector(&value), value))
        .collect::<IndexMap<_, _>>();

    Ok(map)
}

#[cfg(test)]
mod deserialization_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_fullscreen_scene() {
        let json = json!({
            "id": Uuid::new_v4(),
            "enabled": true,
            "duration": "5s",
            "kind": "fullscreen",
            "widgets": [{
                "id": Uuid::new_v4(),
                "row": 0,
                "col": 0,
                "size": "full",
                "kind": "clock",
                "params": {
                    "clock_style": "analog_round",
                    "numbers_font_style": "light",
                    "show_date": false,
                    "show_seconds": false,
                    "show_timezone": false,
                    "timezone": "Europe/Bratislava"
                }
            }]
        });

        let result = serde_json::from_value::<Scene>(json);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn deserialize_combined_scene() {
        let json = json!({
            "id": Uuid::new_v4(),
            "enabled": true,
            "duration": "5s",
            "kind": "combined",
            "widgets": [{
                "id": Uuid::new_v4(),
                "row": 0,
                "col": 0,
                "size": "small",
                "kind": "clock",
                "params": {
                    "clock_style": "analog_rect",
                    "numbers_font_style": "light",
                    "show_date": false,
                    "show_seconds": false,
                    "show_timezone": false,
                    "timezone": "Europe/Bratislava"
                }
            }, {
                "id": Uuid::new_v4(),
                "row": 0,
                "col": 1,
                "size": "medium",
                "kind": "clock",
                "params": {
                    "clock_style": "digital",
                    "numbers_font_style": "medium",
                    "show_date": true,
                    "show_seconds": true,
                    "show_timezone": true,
                    "timezone": "Europe/Prague"
                }
            }]
        });

        let result = serde_json::from_value::<Scene>(json);
        assert!(result.is_ok(), "{result:?}");
    }
}

#[cfg(test)]
mod validate_widget_placement_tests {
    use super::*;

    #[test]
    fn in_bounds() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 1, col: 3 },
            size: WidgetSize::Small,
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn out_of_bounds_row_full() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 2, col: 0 },
            size: WidgetSize::Small,
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::OutOfBounds)
        ));
    }

    #[test]
    fn out_of_bounds_row_partial() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 1, col: 0 },
            size: WidgetSize::Large,
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::OutOfBounds)
        ));
    }

    #[test]
    fn out_of_bounds_col_full() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 0, col: 4 },
            size: WidgetSize::Small,
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::OutOfBounds)
        ));
    }

    #[test]
    fn out_of_bounds_col_partial() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 0, col: 3 },
            size: WidgetSize::Medium,
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::OutOfBounds)
        ));
    }

    #[test]
    fn no_overlap() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 0, col: 0 },
            size: WidgetSize::Small,
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        let mut other_widget = widget.clone();
        other_widget.position.col = 1;

        let result = Scene::validate_widget_placement(&widget, &[other_widget]);
        assert!(result.is_ok());
    }

    #[test]
    fn overlap() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 0, col: 0 },
            size: WidgetSize::Small,
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        let other_widget = widget.clone();

        let result = Scene::validate_widget_placement(&widget, &[other_widget]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::Overlap)
        ));
    }
}

#[cfg(test)]
mod widget_overlaps_tests {
    use super::*;

    #[test]
    fn no_overlap() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 1, col: 1 }, // intentional
            size: WidgetSize::Small,                     // intentional
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        for row_offset in -1..=1_i8 {
            for col_offset in -1..=1_i8 {
                // we want to check positions around the widget
                if row_offset == 0 && col_offset == 0 {
                    continue;
                }

                let mut other_widget = widget.clone();
                other_widget.position.row = widget
                    .position
                    .row
                    .checked_add_signed(row_offset)
                    .expect("BUG: widget has incorrect position (row)");

                other_widget.position.col = widget
                    .position
                    .col
                    .checked_add_signed(col_offset)
                    .expect("BUG: widget has incorrect position (col)");

                assert!(
                    !widget.overlaps(&other_widget),
                    "Failed combination: widget_position={:?}, other_widget_position={:?}",
                    widget.position,
                    other_widget.position
                );
            }
        }
    }

    #[test]
    fn overlap() {
        let widget = Widget {
            id: WidgetId::generate(),
            position: WidgetPosition { row: 1, col: 1 }, // intentional
            size: WidgetSize::Large,                     // intentional
            kind: WidgetKind::Clock(ClockWidget::default()),
        };

        for row_offset in -1..=1_i8 {
            for col_offset in -1..=1_i8 {
                let mut other_widget = widget.clone();
                other_widget.position.row = widget
                    .position
                    .row
                    .checked_add_signed(row_offset)
                    .expect("BUG: widget has incorrect position (row)");

                other_widget.position.col = widget
                    .position
                    .col
                    .checked_add_signed(col_offset)
                    .expect("BUG: widget has incorrect position (col)");

                assert!(
                    widget.overlaps(&other_widget),
                    "Failed combination: widget_position={:?}, other_widget_position={:?}",
                    widget.position,
                    other_widget.position
                );
            }
        }
    }
}

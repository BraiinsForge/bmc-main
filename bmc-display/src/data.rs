// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated;
use crate::indexmap_model::IndexMapModel;
use anyhow::anyhow;
use bmc_shared_time::time::Timezone;
use chrono::{DateTime, TimeDelta, Utc};
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
pub struct SceneCycling {
    pub automatic_cycling_enabled: bool,
    #[serde(with = "humantime_serde")]
    pub automatic_cycling_default_duration: Duration,
    pub transition: SceneCyclingTransition,
}

impl Default for SceneCycling {
    fn default() -> Self {
        Self {
            automatic_cycling_enabled: true,
            automatic_cycling_default_duration: Duration::from_secs(30),
            transition: SceneCyclingTransition::Slide,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SceneCyclingTransition {
    Slide,
    Fade,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
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
#[serde(rename_all = "snake_case")]
pub enum TickerTimeFrame {
    Day1,
    Week1,
    Week2,
    Month1,
    Month3,
    Month6,
    Year1,
    Year2,
    Year5,
    All,
}

impl Display for TickerTimeFrame {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Day1 => "1d",
            Self::Week1 => "1w",
            Self::Week2 => "2w",
            Self::Month1 => "1m",
            Self::Month3 => "3m",
            Self::Month6 => "6m",
            Self::Year1 => "1y",
            Self::Year2 => "2y",
            Self::Year5 => "5y",
            Self::All => "all",
        };

        f.write_str(value)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TickerBtcWidget {
    pub time_frame: TickerTimeFrame,
}

impl Default for TickerBtcWidget {
    fn default() -> Self {
        Self {
            time_frame: TickerTimeFrame::Day1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockHeightWidget {
    pub show_timestamp: bool,
    pub numbers_font_style: FontStyle,
}

impl Default for BlockHeightWidget {
    fn default() -> Self {
        Self {
            show_timestamp: true,
            numbers_font_style: FontStyle::Bold,
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolStyle {
    Overview,
    BigChart,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolChartTimeFrame {
    Hours4,
    Hours12,
    Hours24,
    Days7,
}

impl From<PoolChartTimeFrame> for TimeDelta {
    fn from(value: PoolChartTimeFrame) -> Self {
        match value {
            PoolChartTimeFrame::Hours4 => TimeDelta::hours(4),
            PoolChartTimeFrame::Hours12 => TimeDelta::hours(12),
            PoolChartTimeFrame::Hours24 => TimeDelta::hours(24),
            PoolChartTimeFrame::Days7 => TimeDelta::days(7),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BraiinsPoolWidget {
    pub pool_style: PoolStyle,
    pub chart_frame: PoolChartTimeFrame,
    // pub worker_states: bool,
    pub account_id: Option<AccountId>,
}

impl Default for BraiinsPoolWidget {
    fn default() -> Self {
        Self {
            pool_style: PoolStyle::Overview,
            chart_frame: PoolChartTimeFrame::Hours24,
            // worker_states: true,
            account_id: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteImageWidget {
    pub url: String,
    #[serde(with = "humantime_serde")]
    pub refresh_duration: Duration,
}

impl Default for RemoteImageWidget {
    fn default() -> Self {
        Self {
            url: String::new(),
            refresh_duration: Duration::from_secs(60 * 60 * 24),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RemoteWidgetAssets {
    pub icon: String,
}

#[derive(Debug, Deserialize)]
pub struct RemoteWidgetMetadata {
    pub name: String,
    pub description: String,
    pub assets: RemoteWidgetAssets,
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteWidget {
    pub name: String,
    pub description: String,
    // Base URL plus `/{widget_id}`
    pub widget_url: String,
    // Full URL with icon image
    pub icon_url: String,
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "params")]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    Clock(ClockWidget),
    TickerBtc(TickerBtcWidget),
    BlockHeight(BlockHeightWidget),
    BraiinsPool(BraiinsPoolWidget),
    RemoteImage(RemoteImageWidget),
    BlockchainData,
    RemoteWidget(RemoteWidget),
}

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
    pub fn new(kind: WidgetKind, position: WidgetPosition, size: WidgetSize) -> Self {
        Self {
            id: WidgetId::generate(),
            kind,
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
        let bottom = u32::from(self.position.row + self.size.row_span());
        let right = u32::from(self.position.col + self.size.col_span());

        (bottom <= WidgetPosition::MAX_ROWS) && (right <= WidgetPosition::MAX_COLS)
    }

    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
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
    const DEFAULT_CYCLE_DURATION: Option<Duration> = None;
    const DEFAULT_ENABLED: bool = true;

    #[must_use]
    pub fn fullscreen(widget_kind: WidgetKind) -> Self {
        Self {
            id: SceneId::generate(),
            enabled: Self::DEFAULT_ENABLED,
            cycle_duration: Self::DEFAULT_CYCLE_DURATION,
            kind: SceneKind::Fullscreen,
            widgets: {
                let widget = Widget::new(
                    widget_kind,
                    WidgetPosition { row: 0, col: 0 },
                    WidgetSize::Full,
                );

                indexmap! {
                    widget.id.clone() => widget
                }
            },
        }
    }

    #[must_use]
    pub fn combined() -> Self {
        Self {
            id: SceneId::generate(),
            enabled: Self::DEFAULT_ENABLED,
            cycle_duration: Self::DEFAULT_CYCLE_DURATION,
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

        let new_widget = Widget::new(kind, position, size);

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
        if !widget.in_bounds() {
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
pub enum InitScreen {
    SetupStart,
    SetupWifiConnecting,
    SetupWifiConnected,
    SetupWifiError,
    SetupGeneralError,
    SetupConnectInfo,
    SetupCompleted,
}

impl From<Option<InitScreen>> for generated::InitScreen {
    fn from(value: Option<InitScreen>) -> Self {
        match value {
            None => Self::None,
            Some(InitScreen::SetupStart) => Self::StartConnect,
            Some(InitScreen::SetupWifiConnecting) => Self::WifiConnectProgress,
            Some(InitScreen::SetupWifiConnected) => Self::WifiConnectSuccess,
            Some(InitScreen::SetupWifiError) => Self::WifiConnectFailed,
            Some(InitScreen::SetupGeneralError) => Self::GeneralError,
            Some(InitScreen::SetupConnectInfo) => Self::DeviceSetupQr,
            Some(InitScreen::SetupCompleted) => Self::SetupSuccess,
        }
    }
}

#[derive(Debug)]
pub enum ConnectInfoScreen {
    ConnectInfo,
    WifiConnectProgress,
    WifiConnectFailed,
}

impl From<Option<ConnectInfoScreen>> for generated::ConnectInfoScreen {
    fn from(value: Option<ConnectInfoScreen>) -> Self {
        match value {
            None => Self::None,
            Some(ConnectInfoScreen::ConnectInfo) => Self::ConnectInfo,
            Some(ConnectInfoScreen::WifiConnectProgress) => Self::WifiConnectProgress,
            Some(ConnectInfoScreen::WifiConnectFailed) => Self::WifiConnectFailed,
        }
    }
}

#[derive(Debug)]
pub enum UpgradeScreen {
    DownloadFirmware,
    Upgrade,
    UpgradeFailed,
    UpgradeSuccess,
}

impl From<Option<UpgradeScreen>> for generated::UpgradeScreen {
    fn from(value: Option<UpgradeScreen>) -> Self {
        match value {
            None => Self::None,
            Some(UpgradeScreen::DownloadFirmware) => Self::Download,
            Some(UpgradeScreen::Upgrade) => Self::Progress,
            Some(UpgradeScreen::UpgradeFailed) => Self::Failed,
            Some(UpgradeScreen::UpgradeSuccess) => Self::Success,
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

impl From<generated::WidgetSize> for WidgetSize {
    fn from(value: generated::WidgetSize) -> Self {
        match value {
            generated::WidgetSize::Small => Self::Small,
            generated::WidgetSize::Medium => Self::Medium,
            generated::WidgetSize::Large => Self::Large,
            generated::WidgetSize::Full => Self::Full,
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
            WidgetKind::TickerBtc(config) => {
                slint_widget.kind = generated::WidgetKind::TickerBtc;
                slint_widget.ticker_btc = generated::WidgetBtcData {
                    config: config.into(),
                    ..generated::WidgetBtcData::default()
                }
            }
            WidgetKind::BlockHeight(config) => {
                slint_widget.kind = generated::WidgetKind::BlockHeight;
                slint_widget.blockheight = generated::WidgetBlockHeightData {
                    config: config.into(),
                }
            }
            WidgetKind::BraiinsPool(config) => {
                slint_widget.kind = generated::WidgetKind::BraiinsPool;
                slint_widget.braiins_pool = generated::WidgetBraiinsPoolData {
                    config: config.into(),
                    ..generated::WidgetBraiinsPoolData::default()
                }
            }
            WidgetKind::RemoteImage(_config) => {
                slint_widget.kind = generated::WidgetKind::RemoteImage;
                slint_widget.remote_image = generated::WidgetRemoteImageData::default();
            }
            WidgetKind::BlockchainData => {
                slint_widget.kind = generated::WidgetKind::BlockchainData;
            }
            WidgetKind::RemoteWidget(_config) => {
                slint_widget.kind = generated::WidgetKind::RemoteWidget;
                slint_widget.remote_widget = generated::RemoteWidgetData::default();
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
            numbers_font_style: font_style(from.numbers_font_style),
            show_date: from.show_date,
            show_seconds: from.show_seconds,
            show_timezone: from.show_timezone,
        }
    }
}

impl From<TickerBtcWidget> for generated::WidgetBtcConfig {
    fn from(value: TickerBtcWidget) -> Self {
        Self {
            time_frame: match value.time_frame {
                TickerTimeFrame::Day1 => generated::TickerTimeFrame::Day1,
                TickerTimeFrame::Week1 => generated::TickerTimeFrame::Week1,
                TickerTimeFrame::Week2 => generated::TickerTimeFrame::Week2,
                TickerTimeFrame::Month1 => generated::TickerTimeFrame::Month1,
                TickerTimeFrame::Month3 => generated::TickerTimeFrame::Month3,
                TickerTimeFrame::Month6 => generated::TickerTimeFrame::Month6,
                TickerTimeFrame::Year1 => generated::TickerTimeFrame::Year1,
                TickerTimeFrame::Year2 => generated::TickerTimeFrame::Year2,
                TickerTimeFrame::Year5 => generated::TickerTimeFrame::Year5,
                TickerTimeFrame::All => generated::TickerTimeFrame::All,
            },
        }
    }
}

impl From<BlockHeightWidget> for generated::WidgetBlockHeightConfig {
    fn from(value: BlockHeightWidget) -> Self {
        Self {
            show_timestamp: value.show_timestamp,
            numbers_font_style: font_style(value.numbers_font_style),
        }
    }
}

impl From<BraiinsPoolWidget> for generated::WidgetBraiinsPoolConfig {
    fn from(value: BraiinsPoolWidget) -> Self {
        Self {
            chart_frame: match value.chart_frame {
                PoolChartTimeFrame::Hours4 => generated::ChartFrame::Hours4,
                PoolChartTimeFrame::Hours12 => generated::ChartFrame::Hours12,
                PoolChartTimeFrame::Hours24 => generated::ChartFrame::Hours24,
                PoolChartTimeFrame::Days7 => generated::ChartFrame::Days7,
            },
            pool_style: match value.pool_style {
                PoolStyle::Overview => generated::BraiinsPoolStyle::Overview,
                PoolStyle::BigChart => generated::BraiinsPoolStyle::BigChart,
            },
            // worker_states: value.worker_states,
        }
    }
}

impl From<Scene> for generated::Scene {
    fn from(value: Scene) -> Self {
        // NOTE: value -1 is used as sentinel value to signal that we should use default
        // value from SceneCyclingAdapter
        let cycle_duration = value.cycle_duration.map_or(-1, |duration| {
            #[expect(clippy::cast_possible_truncation)]
            let duration = duration.as_millis() as i64;
            duration
        });

        let widgets = value
            .widgets
            .into_iter()
            .map(|(id, widget)| (id, widget.into()))
            .collect::<IndexMapModel<_, _>>();

        Self {
            id: value.id.to_shared_string(),
            enabled: value.enabled,
            cycle_duration,
            widgets: ModelRc::new(widgets),
        }
    }
}

fn font_style(font_style: FontStyle) -> generated::FontStyle {
    match font_style {
        FontStyle::Light => generated::FontStyle::Light,
        FontStyle::Medium => generated::FontStyle::Medium,
        FontStyle::Bold => generated::FontStyle::Bold,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct AccountId(String);

impl AccountId {
    pub(crate) fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Display for AccountId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AccountId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Err(anyhow!("Empty string"))
        } else {
            Ok(Self(value.to_owned()))
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    BraiinsPool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationType {
    ApiKey(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub r#type: AccountType,
    pub name: String,
    pub authentication: AuthenticationType,
    pub created_at: DateTime<Utc>,
}

impl Account {
    #[must_use]
    pub fn new(account_type: AccountType, name: &str, authentication: AuthenticationType) -> Self {
        Self {
            id: AccountId::generate(),
            r#type: account_type,
            name: name.to_owned(),
            authentication,
            created_at: Utc::now(),
        }
    }
}

#[inline]
pub fn serialize_accounts<S: Serializer>(
    map: &IndexMap<AccountId, Account>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(map.values())
}

#[inline]
pub fn deserialize_accounts<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<IndexMap<AccountId, Account>, D::Error> {
    de_indexmap(deserializer, |account: &Account| account.id.clone())
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
            "cycle_duration": "5s",
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
            "cycle_duration": "5s",
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
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 1, col: 3 },
            WidgetSize::Small,
        );

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn out_of_bounds_row_full() {
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 2, col: 0 },
            WidgetSize::Small,
        );

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::OutOfBounds)
        ));
    }

    #[test]
    fn out_of_bounds_row_partial() {
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 1, col: 0 },
            WidgetSize::Large,
        );

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::OutOfBounds)
        ));
    }

    #[test]
    fn out_of_bounds_col_full() {
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 0, col: 4 },
            WidgetSize::Small,
        );

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::OutOfBounds)
        ));
    }

    #[test]
    fn out_of_bounds_col_partial() {
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 0, col: 3 },
            WidgetSize::Medium,
        );

        let result = Scene::validate_widget_placement(&widget, &[]);
        assert!(matches!(
            result.err(),
            Some(InvalidWidgetPlacementError::OutOfBounds)
        ));
    }

    #[test]
    fn no_overlap() {
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 0, col: 0 },
            WidgetSize::Small,
        );

        let mut other_widget = widget.clone();
        other_widget.position.col = 1;

        let result = Scene::validate_widget_placement(&widget, &[other_widget]);
        assert!(result.is_ok());
    }

    #[test]
    fn overlap() {
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 0, col: 0 },
            WidgetSize::Small,
        );

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
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 1, col: 1 }, // intentional
            WidgetSize::Small,                 // intentional
        );

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
        let widget = Widget::new(
            WidgetKind::Clock(ClockWidget::default()),
            WidgetPosition { row: 1, col: 1 }, // intentional
            WidgetSize::Large,                 // intentional
        );

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

#[derive(Debug)]
pub enum SignalStrength {
    Offline,
    Low,
    Fair,
    Strong,
}

impl From<SignalStrength> for generated::SignalStrength {
    fn from(value: SignalStrength) -> Self {
        match value {
            SignalStrength::Offline => Self::Offline,
            SignalStrength::Low => Self::Low,
            SignalStrength::Fair => Self::Fair,
            SignalStrength::Strong => Self::Strong,
        }
    }
}

// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::{
    alarm::{AlarmData, AlarmId},
    utils::replace_file,
};
use anyhow::{Context, Result, bail};
use bmc_display::data::{
    Account, AccountId, BlockHeightWidget, ClockStyle, ClockWidget, Scene, SceneCycling, SceneId,
    SceneKind, TickerBtcWidget, Widget, WidgetKind, WidgetPosition, WidgetSize,
    deserialize_accounts, deserialize_scenes, serialize_accounts, serialize_scenes,
};
use bmc_shared_time::time::{DateFormat, TimeSystem, Timezone, WeekDay};
use bmc_shared_utils::number_format::NumberFormat;
use bmc_upgrade::autoupgrade::AutoUpgradeConfig;
use chrono::{Local, NaiveTime};
use indexmap::{IndexMap, indexmap};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::warn;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(
        serialize_with = "serialize_scenes",
        deserialize_with = "deserialize_scenes",
        default
    )]
    pub scenes: IndexMap<SceneId, Scene>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scene_cycling: Option<SceneCycling>,
    #[serde(skip_serializing_if = "Option::is_none")]
    localization: Option<LocalizationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_collection: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    night_mode: Option<NightModeConfigData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sound_volume_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alarms: Option<Vec<AlarmData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    led_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    autoupgrade: Option<AutoUpgradeConfig>,
    #[serde(
        default,
        serialize_with = "serialize_accounts",
        deserialize_with = "deserialize_accounts"
    )]
    pub accounts: IndexMap<AccountId, Account>,
}

impl Config {
    pub fn scene_cycling(&self) -> SceneCycling {
        self.scene_cycling.clone().unwrap_or_default()
    }

    pub fn set_scene_cycling(&mut self, config: SceneCycling) {
        self.scene_cycling = Some(config);
    }

    pub fn localization_config(&self) -> LocalizationConfig {
        self.localization.clone().unwrap_or_default()
    }

    pub fn set_time_system(&mut self, time_system: TimeSystem) {
        self.localization.get_or_insert_default().time_system = time_system;
    }

    pub fn set_number_format(&mut self, number_format: NumberFormat) {
        self.localization.get_or_insert_default().number_format = number_format;
    }

    pub fn set_date_format(&mut self, date_format: DateFormat) {
        self.localization.get_or_insert_default().date_format = date_format;
    }

    pub fn data_collection(&self) -> bool {
        self.data_collection.unwrap_or_default()
    }

    pub fn set_data_collection(&mut self, data_collection: bool) {
        self.data_collection = Some(data_collection);
    }

    pub fn set_first_day_of_week(&mut self, day: WeekDay) {
        self.localization.get_or_insert_default().first_day_of_week = day;
    }

    pub fn set_temperature_unit(&mut self, temperature_unit: TemperatureUnit) {
        self.localization.get_or_insert_default().temperature_unit = temperature_unit;
    }

    pub fn set_unit_system(&mut self, unit_system: UnitSystem) {
        self.localization.get_or_insert_default().unit_system = unit_system;
    }

    pub fn show_seconds_in_status_bar(&mut self, show: bool) {
        self.localization
            .get_or_insert_default()
            .show_seconds_in_status_bar = show;
    }

    pub fn led_enabled(&self) -> bool {
        self.led_enabled.unwrap_or(true)
    }

    pub fn set_led_enabled(&mut self, led_enabled: bool) {
        self.led_enabled = Some(led_enabled);
    }

    pub fn autoupgrade(&self) -> AutoUpgradeConfig {
        self.autoupgrade.clone().unwrap_or_default()
    }

    pub fn set_autoupgrade(&mut self, config: AutoUpgradeConfig) {
        self.autoupgrade = Some(config);
    }

    fn validate(&self) -> Result<()> {
        self.validate_scenes()
    }

    fn validate_scenes(&self) -> Result<()> {
        for scene in self.scenes.values() {
            if scene
                .cycle_duration
                .is_some_and(|duration| duration < Scene::MIN_CYCLE_DURATION)
            {
                bail!("Duration for scene `{}` is too short", scene.id);
            }

            match scene.kind {
                SceneKind::Fullscreen => {
                    if scene.widgets.len() != 1 {
                        bail!(
                            "Fullscreen scene `{}` does not have exactly one widget",
                            scene.id
                        );
                    }

                    let widget = &scene.widgets[0];
                    if widget.position.row != 0 || widget.position.col != 0 {
                        bail!(
                            "Fullscreen scene `{}` has widget `{}` with incorrect position (expected row=0, col=0)",
                            scene.id,
                            widget.id
                        );
                    }
                    if widget.size != WidgetSize::Full {
                        bail!(
                            "Fullscreen scene `{}` has widget `{}` with incorrect size (expected `full`)",
                            scene.id,
                            widget.id
                        );
                    }
                }
                SceneKind::Combined => {
                    for widget in scene.widgets.values() {
                        if widget.size == WidgetSize::Full {
                            bail!(
                                "Combined scene `{}` has widget `{}` with incorrect size (expected `small`, `medium` or `large`)",
                                scene.id,
                                widget.id
                            );
                        }

                        if !widget.in_bounds() {
                            bail!(
                                "Combined scene `{}` has widget `{}` which is out of bounds (position + size)",
                                scene.id,
                                widget.id
                            );
                        }
                    }

                    for (widget, other_widget) in scene.widgets.values().tuple_combinations() {
                        if widget.overlaps(other_widget) {
                            bail!(
                                "Combined scene `{}` has widget `{}` which overlaps with widget `{}`",
                                scene.id,
                                widget.id,
                                other_widget.id
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let config_data = fs::read_to_string(path)
            .await
            .context("Failed to read config file")?;

        let config: Self =
            serde_json::from_str(config_data.as_str()).context("Failed to deserialize config")?;

        config.validate().context("Config validation failed")?;

        Ok(config)
    }

    async fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let config_data =
            serde_json::to_string_pretty(&self).context("Failed to serialize config")?;

        replace_file(path, config_data.as_bytes())
            .await
            .context("Failed to replace config file")?;

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        let clock_fullscreen_scene = Scene::fullscreen(WidgetKind::Clock(ClockWidget::default()));
        let ticker_btc_fullscreen_scene =
            Scene::fullscreen(WidgetKind::TickerBtc(TickerBtcWidget::default()));

        let combined_scene = {
            let mut scene = Scene::combined();

            let clock_widget = Widget::new(
                WidgetKind::Clock({
                    ClockWidget {
                        clock_style: ClockStyle::AnalogRect,
                        ..ClockWidget::default()
                    }
                }),
                WidgetPosition { row: 0, col: 0 },
                WidgetSize::Medium,
            );

            let block_height_widget = Widget::new(
                WidgetKind::BlockHeight(BlockHeightWidget::default()),
                WidgetPosition { row: 1, col: 0 },
                WidgetSize::Medium,
            );

            let ticker_btc_widget = Widget::new(
                WidgetKind::TickerBtc(TickerBtcWidget::default()),
                WidgetPosition { row: 0, col: 2 },
                WidgetSize::Large,
            );

            scene.widgets = indexmap! {
                clock_widget.id.clone() => clock_widget,
                block_height_widget.id.clone() => block_height_widget,
                ticker_btc_widget.id.clone() => ticker_btc_widget,
            };

            scene
        };

        let scenes = indexmap! {
            clock_fullscreen_scene.id.clone() => clock_fullscreen_scene,
            ticker_btc_fullscreen_scene.id.clone() => ticker_btc_fullscreen_scene,
            combined_scene.id.clone() => combined_scene,
        };

        Self {
            scenes,
            scene_cycling: None,
            localization: None,
            data_collection: None,
            brightness_pct: None,
            night_mode: None,
            sound_volume_pct: None,
            alarms: None,
            led_enabled: None,
            autoupgrade: None,
            accounts: indexmap! {},
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LocalizationConfig {
    pub time_system: TimeSystem,
    pub number_format: NumberFormat,
    pub date_format: DateFormat,
    pub first_day_of_week: WeekDay,
    pub show_seconds_in_status_bar: bool,
    pub temperature_unit: TemperatureUnit,
    pub unit_system: UnitSystem,
}

#[derive(Clone, Debug)]
pub struct ConfigHandle {
    path: PathBuf,
    config: Config,
    default_brightness_pct: u8,
    default_night_mode_brightness_pct: u8,
    default_sound_volume_pct: u8,
    default_night_mode_sound_volume_pct: u8,
}

impl ConfigHandle {
    pub async fn init(
        path: PathBuf,
        default_brightness_pct: u8,
        default_night_mode_brightness_pct: u8,
        default_sound_volume_pct: u8,
        default_night_mode_sound_volume_pct: u8,
    ) -> Self {
        let config = match Config::load(&path).await {
            Ok(config) => config,
            Err(err) => {
                warn!(?err, "Failed to load config. Replacing with default config");

                let mut default_config = Config::default();

                if let Err(err) = default_config.save(&path).await {
                    warn!(?err, "Failed to save default config");
                }

                default_config
            }
        };

        Self {
            path,
            config,
            default_brightness_pct,
            default_night_mode_brightness_pct,
            default_sound_volume_pct,
            default_night_mode_sound_volume_pct,
        }
    }

    pub async fn save(&mut self) -> Result<()> {
        self.config.save(&self.path).await
    }

    pub fn set_brightness(&mut self, brightness_pct: u8) {
        self.brightness_pct = Some(brightness_pct);
    }

    pub fn brightness_pct(&self) -> u8 {
        self.brightness_pct.unwrap_or(self.default_brightness_pct)
    }

    pub fn night_mode(&self) -> NightModeConfig {
        self.night_mode
            .clone()
            .unwrap_or_default()
            .into_night_mode_config(
                self.default_night_mode_brightness_pct,
                self.default_night_mode_sound_volume_pct,
            )
    }

    pub fn set_night_mode_enabled(&mut self, enabled: bool) {
        self.night_mode.get_or_insert_default().enabled = enabled;
    }

    pub fn set_night_mode_brightness(&mut self, brightness_pct: u8) {
        self.night_mode.get_or_insert_default().brightness_pct = Some(brightness_pct);
    }

    pub fn set_night_mode_interval(&mut self, from: NaiveTime, to: NaiveTime) {
        let night_mode = self.night_mode.get_or_insert_default();
        night_mode.from = from;
        night_mode.to = to;
    }

    pub fn set_night_mode_sound_volume(&mut self, sound_volume_pct: u8) {
        self.night_mode.get_or_insert_default().sound_volume_pct = Some(sound_volume_pct);
    }

    pub fn set_sound_volume(&mut self, sound_volume_pct: u8) {
        self.sound_volume_pct = Some(sound_volume_pct);
    }

    pub fn sound_volume_pct(&self) -> u8 {
        self.sound_volume_pct
            .unwrap_or(self.default_sound_volume_pct)
    }

    pub fn alarms(&self) -> Vec<AlarmData> {
        self.alarms.clone().unwrap_or_default()
    }

    pub fn add_alarm(&mut self, alarm: AlarmData) {
        self.alarms.get_or_insert_default().push(alarm);
    }

    pub fn remove_alarm(&mut self, id: &AlarmId) {
        if let Some(pos) = self
            .alarms
            .as_ref()
            .and_then(|alarms| alarms.iter().position(|x| x.id == *id))
        {
            if let Some(alarms) = self.alarms.as_mut() {
                alarms.remove(pos);
            }
        }
    }

    pub fn set_alarm(&mut self, alarm: AlarmData) {
        if let Some(item) = self
            .alarms
            .as_mut()
            .and_then(|alarms| alarms.iter_mut().find(|x| x.id == alarm.id))
        {
            *item = alarm;
        }
    }
}

impl Deref for ConfigHandle {
    type Target = Config;

    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

impl DerefMut for ConfigHandle {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.config
    }
}

impl AsRef<Config> for ConfigHandle {
    fn as_ref(&self) -> &Config {
        self
    }
}

impl AsMut<Config> for ConfigHandle {
    fn as_mut(&mut self) -> &mut Config {
        self
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct NightModeConfigData {
    enabled: bool,
    from: NaiveTime,
    to: NaiveTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sound_volume_pct: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct NightModeConfig {
    pub enabled: bool,
    pub from: NaiveTime,
    pub to: NaiveTime,
    pub brightness_pct: u8,
    pub sound_volume_pct: u8,
}

impl NightModeConfig {
    pub fn is_active(&self, timezone: &Timezone) -> bool {
        let now = Local::now().with_timezone(timezone.chrono()).time();

        self.enabled && Self::is_time_in_range(self.from, self.to, now)
    }

    /// Checks whether `now` (in local time) is in the [from, to) range.
    /// Handles ranges that cross midnight.
    pub fn is_time_in_range(from: NaiveTime, to: NaiveTime, now: NaiveTime) -> bool {
        if from <= to {
            now >= from && now < to
        } else {
            now >= from || now < to
        }
    }
}

impl NightModeConfigData {
    fn default_from() -> NaiveTime {
        NaiveTime::from_hms_opt(22, 30, 0).expect("BUG: Invalid default night mode interval")
    }

    fn default_to() -> NaiveTime {
        NaiveTime::from_hms_opt(6, 30, 0).expect("BUG: Invalid default night mode interval")
    }

    fn into_night_mode_config(
        self,
        default_brightness: u8,
        default_sound_volume: u8,
    ) -> NightModeConfig {
        NightModeConfig {
            enabled: self.enabled,
            from: self.from,
            to: self.to,
            brightness_pct: self.brightness_pct.unwrap_or(default_brightness),
            sound_volume_pct: self.sound_volume_pct.unwrap_or(default_sound_volume),
        }
    }
}

impl Default for NightModeConfigData {
    fn default() -> Self {
        Self {
            enabled: false,
            from: NightModeConfigData::default_from(),
            to: NightModeConfigData::default_to(),
            brightness_pct: None,
            sound_volume_pct: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum UnitSystem {
    #[default]
    Metric,
    Imperial,
}

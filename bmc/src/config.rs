// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::{Account, AccountId, SceneCycling, deserialize_accounts, serialize_accounts};
use crate::{
    alarm::{AlarmData, AlarmId},
    scene::{Scene, SceneId, SceneKind, WidgetSize, deserialize_scenes, serialize_scenes},
    utils::replace_file,
};
use anyhow::{Context, Result, bail};
use bmc_shared_time::time::{DateFormat, TimeSystem, Timezone, WeekDay};
use bmc_shared_utils::number_format::NumberFormat;
use bmc_shared_utils::temperature::TemperatureUnit;
use bmc_shared_utils::unit_system::UnitSystem;
use bmc_upgrade::autoupgrade::AutoUpgradeConfig;
use bmc_widget_manifest::{ParamKey, ParamValue};
use chrono::{Local, NaiveTime};
use indexmap::{IndexMap, indexmap};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::broadcast;
use tracing::warn;
use uuid::Uuid;

const CHANNEL_CAPACITY: usize = 8;

#[derive(Clone, Debug)]
struct ConfigNotify {
    localization: broadcast::Sender<LocalizationConfig>,
    night_mode_schedule: broadcast::Sender<()>,
    led_settings: broadcast::Sender<()>,
    brightness_settings: broadcast::Sender<()>,
    screen_off_timeout: broadcast::Sender<Option<u32>>,
}

impl ConfigNotify {
    fn new() -> Self {
        let (tx_localization, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_night_mode_schedule, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_led_settings, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_brightness_settings, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_screen_off_timeout, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            localization: tx_localization,
            night_mode_schedule: tx_night_mode_schedule,
            led_settings: tx_led_settings,
            brightness_settings: tx_brightness_settings,
            screen_off_timeout: tx_screen_off_timeout,
        }
    }

    fn subscribe_localization_change(&self) -> broadcast::Receiver<LocalizationConfig> {
        self.localization.subscribe()
    }

    fn subscribe_screen_off_timeout_change(&self) -> broadcast::Receiver<Option<u32>> {
        self.screen_off_timeout.subscribe()
    }

    fn localization_changed(&self, config: LocalizationConfig) {
        if let Err(err) = self.localization.send(config) {
            warn!(error = %err, "Failed to send localization changed notification");
        }
    }

    fn subscribe_night_mode_schedule_change(&self) -> broadcast::Receiver<()> {
        self.night_mode_schedule.subscribe()
    }

    fn night_mode_schedule_changed(&self) {
        let _ = self.night_mode_schedule.send(());
    }

    fn subscribe_led_settings_change(&self) -> broadcast::Receiver<()> {
        self.led_settings.subscribe()
    }

    fn led_settings_changed(&self) {
        let _ = self.led_settings.send(());
    }

    fn subscribe_brightness_settings_change(&self) -> broadcast::Receiver<()> {
        self.brightness_settings.subscribe()
    }

    fn brightness_settings_changed(&self) {
        let _ = self.brightness_settings.send(());
    }

    fn screen_off_timeout_changed(&self, timeout: Option<u32>) {
        if let Err(err) = self.screen_off_timeout.send(timeout) {
            warn!(error = %err, "Failed to send screen off timeout changed notification");
        }
    }
}

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
    boot_sound_enabled: Option<bool>,
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

    pub fn boot_sound_enabled(&self) -> bool {
        self.boot_sound_enabled.unwrap_or(true)
    }

    pub fn set_boot_sound_enabled(&mut self, enabled: bool) {
        self.boot_sound_enabled = Some(enabled);
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
    // TODO: Refine default scenes after all widgets are migrated to multi-process system.
    // The original default included: clock fullscreen, ticker_btc fullscreen, and a combined
    // scene with analog clock, block height, and ticker_btc widgets.
    fn default() -> Self {
        // Digital clock widget type ID from widgets/digital-clock/manifest.json
        let digital_clock_type_id =
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").expect("BUG: invalid UUID");

        let digital_clock_scene = Scene::fullscreen(
            digital_clock_type_id,
            params_map(&[
                ("showSeconds", ParamValue::Boolean(true)),
                ("showTimezone", ParamValue::Boolean(true)),
                ("fontStyle", ParamValue::String("medium".into())),
            ])
            .expect("BUG: invalid built-in ParamKey in digital-clock defaults"),
        );

        // Flip clock widget type ID from widgets/flip-clock/manifest.json
        let flip_clock_type_id =
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440002").expect("BUG: invalid UUID");

        let flip_clock_scene = Scene::fullscreen(
            flip_clock_type_id,
            params_map(&[("mode", ParamValue::String("extruded".into()))])
                .expect("BUG: invalid built-in ParamKey in flip-clock defaults"),
        );

        let scenes = indexmap! {
            digital_clock_scene.id => digital_clock_scene,
            flip_clock_scene.id => flip_clock_scene,
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
            boot_sound_enabled: None,
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
    #[serde(default)]
    pub unit_system: UnitSystem,
}

#[derive(Clone, Debug)]
#[expect(clippy::struct_excessive_bools)]
pub struct ConfigHandle {
    path: PathBuf,
    config: Config,
    config_notify: ConfigNotify,
    localization_dirty: bool,
    night_mode_schedule_dirty: bool,
    led_settings_dirty: bool,
    brightness_settings_dirty: bool,
    screen_off_timeout_dirty: bool,
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

                if path.exists() {
                    let backup_path = path.with_extension("json.bcp");
                    if let Err(err) = fs::copy(&path, &backup_path).await {
                        warn!(
                            "Failed to back up config to {}: {err:#}",
                            backup_path.display()
                        );
                    } else {
                        warn!("Backed up broken config to {}", backup_path.display());
                    }
                }

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
            config_notify: ConfigNotify::new(),
            localization_dirty: false,
            night_mode_schedule_dirty: false,
            led_settings_dirty: false,
            brightness_settings_dirty: false,
            screen_off_timeout_dirty: false,
            default_brightness_pct,
            default_night_mode_brightness_pct,
            default_sound_volume_pct,
            default_night_mode_sound_volume_pct,
        }
    }

    pub fn subscribe_localization_change(&self) -> broadcast::Receiver<LocalizationConfig> {
        self.config_notify.subscribe_localization_change()
    }

    pub fn subscribe_night_mode_schedule_change(&self) -> broadcast::Receiver<()> {
        self.config_notify.subscribe_night_mode_schedule_change()
    }

    pub fn subscribe_led_settings_change(&self) -> broadcast::Receiver<()> {
        self.config_notify.subscribe_led_settings_change()
    }

    pub fn subscribe_brightness_settings_change(&self) -> broadcast::Receiver<()> {
        self.config_notify.subscribe_brightness_settings_change()
    }

    pub fn subscribe_screen_off_timeout_change(&self) -> broadcast::Receiver<Option<u32>> {
        self.config_notify.subscribe_screen_off_timeout_change()
    }

    pub async fn save(&mut self) -> Result<()> {
        if self.localization_dirty {
            self.config_notify
                .localization_changed(self.localization_config());
            self.localization_dirty = false;
        }

        if self.night_mode_schedule_dirty {
            self.config_notify.night_mode_schedule_changed();
            self.night_mode_schedule_dirty = false;
        }
        if self.led_settings_dirty {
            self.config_notify.led_settings_changed();
            self.led_settings_dirty = false;
        }
        if self.brightness_settings_dirty {
            self.config_notify.brightness_settings_changed();
            self.brightness_settings_dirty = false;
        }
        if self.screen_off_timeout_dirty {
            let timeout = self.night_mode().screen_off_timeout_secs;
            self.config_notify.screen_off_timeout_changed(timeout);
            self.screen_off_timeout_dirty = false;
        }

        self.config.save(&self.path).await?;

        Ok(())
    }

    pub fn set_time_system(&mut self, time_system: TimeSystem) {
        self.config.set_time_system(time_system);
        self.localization_dirty = true;
    }

    pub fn set_number_format(&mut self, number_format: NumberFormat) {
        self.config.set_number_format(number_format);
        self.localization_dirty = true;
    }

    pub fn set_date_format(&mut self, date_format: DateFormat) {
        self.config.set_date_format(date_format);
        self.localization_dirty = true;
    }

    pub fn set_first_day_of_week(&mut self, day: WeekDay) {
        self.config.set_first_day_of_week(day);
        self.localization_dirty = true;
    }

    pub fn set_temperature_unit(&mut self, temperature_unit: TemperatureUnit) {
        self.config.set_temperature_unit(temperature_unit);
        self.localization_dirty = true;
    }

    pub fn set_unit_system(&mut self, unit_system: UnitSystem) {
        self.config.set_unit_system(unit_system);
        self.localization_dirty = true;
    }

    pub fn set_brightness(&mut self, brightness_pct: u8) {
        self.brightness_pct = Some(brightness_pct);
        self.brightness_settings_dirty = true;
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
        self.night_mode_schedule_dirty = true;
    }

    pub fn set_night_mode_brightness(&mut self, brightness_pct: u8) {
        self.night_mode.get_or_insert_default().brightness_pct = Some(brightness_pct);
        self.brightness_settings_dirty = true;
    }

    pub fn set_night_mode_interval(&mut self, from: NaiveTime, to: NaiveTime) {
        let night_mode = self.night_mode.get_or_insert_default();
        night_mode.from = from;
        night_mode.to = to;
        self.night_mode_schedule_dirty = true;
    }

    pub fn set_night_mode_sound_volume(&mut self, sound_volume_pct: u8) {
        self.night_mode.get_or_insert_default().sound_volume_pct = Some(sound_volume_pct);
    }

    pub fn set_night_mode_led_enabled(&mut self, led_enabled: bool) {
        self.night_mode.get_or_insert_default().led_enabled = Some(led_enabled);
        self.led_settings_dirty = true;
    }

    pub fn set_led_enabled(&mut self, led_enabled: bool) {
        self.config.set_led_enabled(led_enabled);
        self.led_settings_dirty = true;
    }

    pub fn set_night_mode_screen_off_timeout(&mut self, timeout: Option<u32>) {
        self.night_mode
            .get_or_insert_default()
            .screen_off_timeout_secs = timeout;
        self.screen_off_timeout_dirty = true;
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
            && let Some(alarms) = self.alarms.as_mut()
        {
            alarms.remove(pos);
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
    #[serde(skip_serializing_if = "Option::is_none")]
    led_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_off_timeout_secs: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct NightModeConfig {
    pub enabled: bool,
    pub from: NaiveTime,
    pub to: NaiveTime,
    pub brightness_pct: u8,
    pub sound_volume_pct: u8,
    pub led_enabled: bool,
    pub screen_off_timeout_secs: Option<u32>,
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
            led_enabled: self.led_enabled.unwrap_or(false),
            screen_off_timeout_secs: self.screen_off_timeout_secs,
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
            led_enabled: None,
            screen_off_timeout_secs: None,
        }
    }
}

fn params_map(entries: &[(&str, ParamValue)]) -> Result<BTreeMap<ParamKey, ParamValue>, String> {
    entries
        .iter()
        .map(|(k, v)| ParamKey::try_new((*k).to_owned()).map(|key| (key, v.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_constructs_without_panic() {
        let _ = Config::default();
    }

    /// Tempfile-backed `ConfigHandle` for notification tests.
    async fn fresh_handle() -> (tempfile::TempDir, ConfigHandle) {
        let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
        let path = tmp.path().join("bmc-config.json");
        let handle = ConfigHandle::init(path, 50, 50, 50, 50).await;
        (tmp, handle)
    }

    /// Regression guard for the `localization_dirty` cohort — every
    /// `ConfigHandle` setter that mutates a localization field must
    /// flip the dirty flag so `save()` fans the change to subscribers.
    #[tokio::test]
    async fn set_first_day_of_week_via_handle_notifies_subscribers() {
        let (_tmp, mut handle) = fresh_handle().await;
        let mut rx = handle.subscribe_localization_change();

        handle.set_first_day_of_week(WeekDay::Wednesday);
        handle.save().await.expect("BUG: save must succeed");

        let cfg = rx
            .recv()
            .await
            .expect("BUG: subscriber must receive change");
        assert_eq!(cfg.first_day_of_week, WeekDay::Wednesday);
    }

    #[tokio::test]
    async fn set_temperature_unit_via_handle_notifies_subscribers() {
        let (_tmp, mut handle) = fresh_handle().await;
        let mut rx = handle.subscribe_localization_change();

        handle.set_temperature_unit(TemperatureUnit::Fahrenheit);
        handle.save().await.expect("BUG: save must succeed");

        let cfg = rx
            .recv()
            .await
            .expect("BUG: subscriber must receive change");
        assert_eq!(cfg.temperature_unit, TemperatureUnit::Fahrenheit);
    }

    #[tokio::test]
    async fn set_unit_system_via_handle_notifies_subscribers() {
        let (_tmp, mut handle) = fresh_handle().await;
        let mut rx = handle.subscribe_localization_change();

        handle.set_unit_system(UnitSystem::Imperial);
        handle.save().await.expect("BUG: save must succeed");

        let cfg = rx
            .recv()
            .await
            .expect("BUG: subscriber must receive change");
        assert_eq!(cfg.unit_system, UnitSystem::Imperial);
    }
}

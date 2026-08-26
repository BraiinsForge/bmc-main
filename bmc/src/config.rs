// Copyright (C) 2025  Braiins Systems s.r.o.
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

use crate::data::{Account, AccountId, SceneCycling};
use crate::{
    alarm::{AlarmData, AlarmId},
    scene::{Scene, SceneId, SceneKind, WidgetPlacement, deserialize_scenes, serialize_scenes},
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
use indexmap::IndexMap;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::broadcast;
use tracing::{info, warn};

mod defaults;
pub(crate) mod widget_uuids;

const CHANNEL_CAPACITY: usize = 8;

/// Maps a runtime widget instance id (stringified `WidgetId` UUID) to
/// the scene it belongs to. Built by walking every scene's widget list.
/// Re-published on every config save that touched scene structure.
pub type WidgetSceneMap = HashMap<String, SceneId>;

#[derive(Clone, Debug)]
struct ConfigNotify {
    localization: broadcast::Sender<LocalizationConfig>,
    night_mode_schedule: broadcast::Sender<()>,
    led_settings: broadcast::Sender<()>,
    brightness_settings: broadcast::Sender<()>,
    sound_settings: broadcast::Sender<()>,
    screen_off_timeout: broadcast::Sender<Option<u32>>,
    scenes_change: broadcast::Sender<WidgetSceneMap>,
}

impl ConfigNotify {
    fn new() -> Self {
        let (tx_localization, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_night_mode_schedule, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_led_settings, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_brightness_settings, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_sound_settings, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_screen_off_timeout, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        let (tx_scenes_change, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            localization: tx_localization,
            night_mode_schedule: tx_night_mode_schedule,
            led_settings: tx_led_settings,
            brightness_settings: tx_brightness_settings,
            sound_settings: tx_sound_settings,
            screen_off_timeout: tx_screen_off_timeout,
            scenes_change: tx_scenes_change,
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

    fn subscribe_sound_settings_change(&self) -> broadcast::Receiver<()> {
        self.sound_settings.subscribe()
    }

    fn sound_settings_changed(&self) {
        let _ = self.sound_settings.send(());
    }

    fn screen_off_timeout_changed(&self, timeout: Option<u32>) {
        if let Err(err) = self.screen_off_timeout.send(timeout) {
            warn!(error = %err, "Failed to send screen off timeout changed notification");
        }
    }

    fn subscribe_scenes_change(&self) -> broadcast::Receiver<WidgetSceneMap> {
        self.scenes_change.subscribe()
    }

    fn scenes_changed(&self, snapshot: WidgetSceneMap) {
        let _ = self.scenes_change.send(snapshot);
    }
}

/// Current on-disk config schema version.
/// Bump on every breaking shape change and wire
/// a new migration arm in `crate::config_migration`.
///
/// - `0`: slint-monolith schema (`kind`-enum widgets).
/// - `1`: first manifest-driven widget schema.
/// - `2`: accounts become typed credential instances and move out of the config
///   into the secret store beside it (see [`crate::secret_store`]);
///   widget `placement` replaces legacy `size`.
pub const CONFIG_VERSION: u32 = 2;

/// Cap on concurrently running widgets, sized to the device's 256 MB RAM budget.
pub const MAX_RUNNING_WIDGETS: usize = 56;

pub(crate) fn fits_running_widgets(running: usize, additional: usize) -> bool {
    additional <= MAX_RUNNING_WIDGETS.saturating_sub(running)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Schema version of the on-disk config; see [`CONFIG_VERSION`] for the version history. `0` or
    /// missing means a legacy config that needs migration. Unknown (future) values abort the load —
    /// see `crate::config_migration::LoadedConfig`.
    #[serde(default)]
    pub version: u32,
    #[serde(
        serialize_with = "serialize_scenes",
        deserialize_with = "deserialize_scenes",
        default
    )]
    scenes: IndexMap<SceneId, Scene>,
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
}

impl Config {
    pub fn scenes(&self) -> &IndexMap<SceneId, Scene> {
        &self.scenes
    }

    pub fn active_widget_count(&self) -> usize {
        self.scenes
            .values()
            .filter(|scene| scene.enabled)
            .map(|scene| scene.widgets.len())
            .sum()
    }

    pub fn can_activate_widgets(&self, count: usize) -> bool {
        fits_active_widgets(self.active_widget_count(), count)
    }

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

    pub fn widget_scene_map(&self) -> WidgetSceneMap {
        let mut map = WidgetSceneMap::new();
        for (scene_id, scene) in &self.scenes {
            for widget_id in scene.widgets.keys() {
                map.insert(widget_id.as_uuid().to_string(), *scene_id);
            }
        }
        map
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.validate_scenes()?;
        self.validate_scene_cycling()
    }

    fn validate_scene_cycling(&self) -> Result<()> {
        let duration = self.scene_cycling().automatic_cycling_default_duration;
        if duration < Scene::MIN_CYCLE_DURATION {
            bail!(
                "Automatic scene cycling default duration {duration:?} is shorter than the \
                 minimum {:?}",
                Scene::MIN_CYCLE_DURATION
            );
        }
        Ok(())
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
                    if widget.placement != WidgetPlacement::Fullscreen {
                        bail!(
                            "Fullscreen scene `{}` has widget `{}` with incorrect placement (expected `fullscreen`)",
                            scene.id,
                            widget.id
                        );
                    }
                }
                SceneKind::Combined => {
                    for widget in scene.widgets.values() {
                        if widget.placement == WidgetPlacement::Fullscreen {
                            bail!(
                                "Combined scene `{}` has widget `{}` with incorrect placement (expected slot span, not fullscreen)",
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

    async fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        // Belt-and-braces: every on-disk config carries the current
        // schema version. Constructors already set it, but a caller
        // could mutate `self.version` directly — pin it here.
        self.version = CONFIG_VERSION;
        let config_data =
            serde_json::to_string_pretty(&self).context("Failed to serialize config")?;

        replace_file(path, config_data.as_bytes())
            .await
            .context("Failed to replace config file")?;

        Ok(())
    }
}

/// Top-level settings recovered from a v0 config. The shapes are
/// identical on both sides of the migration, so every field is
/// already the current type; `None` means the legacy file lacked
/// the field or its value failed the lenient re-parse (dropped
/// with a warning by the migration).
#[derive(Debug, Default)]
pub(crate) struct MigratedSettings {
    pub scene_cycling: Option<SceneCycling>,
    pub localization: Option<LocalizationConfig>,
    pub data_collection: Option<bool>,
    pub brightness_pct: Option<u8>,
    pub night_mode: Option<NightModeConfigData>,
    pub sound_volume_pct: Option<u8>,
    pub alarms: Option<Vec<AlarmData>>,
    pub led_enabled: Option<bool>,
    pub boot_sound_enabled: Option<bool>,
    pub autoupgrade: Option<AutoUpgradeConfig>,
}

impl Config {
    /// Assemble a current-schema config from what a v0 migration recovers:
    /// the scene layout, plus the top-level settings whose shape survived unchanged
    /// (a `None` falls back to the field-less default).
    /// `version` is pinned to [`CONFIG_VERSION`] so the result serialises as current.
    pub(crate) fn from_migrated_parts(
        scenes: IndexMap<SceneId, Scene>,
        settings: MigratedSettings,
    ) -> Self {
        // Destructure so a settings field added later cannot be
        // silently forgotten here.
        let MigratedSettings {
            scene_cycling,
            localization,
            data_collection,
            brightness_pct,
            night_mode,
            sound_volume_pct,
            alarms,
            led_enabled,
            boot_sound_enabled,
            autoupgrade,
        } = settings;
        Self {
            version: CONFIG_VERSION,
            scenes,
            scene_cycling,
            localization,
            data_collection,
            brightness_pct,
            night_mode,
            sound_volume_pct,
            alarms,
            led_enabled,
            boot_sound_enabled,
            autoupgrade,
        }
    }

    #[must_use]
    pub fn platform_default(product: bmc_platform::Product) -> Self {
        Self {
            version: CONFIG_VERSION,
            scenes: defaults::scenes_for(product),
            scene_cycling: Some(SceneCycling::default()),
            localization: None,
            data_collection: None,
            brightness_pct: None,
            night_mode: None,
            sound_volume_pct: None,
            alarms: None,
            led_enabled: None,
            boot_sound_enabled: None,
            autoupgrade: None,
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
    /// Set when boot migrated a legacy config in memory but has not yet
    /// written it back. The migrated config is persisted only on the
    /// first genuine change, so the on-disk file keeps its original
    /// version until then (leaving it readable by an older BMC
    /// application). Cleared once that first save commits the upgrade.
    migrated: bool,
    localization_dirty: bool,
    night_mode_schedule_dirty: bool,
    led_settings_dirty: bool,
    brightness_settings_dirty: bool,
    sound_settings_dirty: bool,
    screen_off_timeout_dirty: bool,
    scenes_dirty: bool,
    default_brightness_pct: u8,
    default_night_mode_brightness_pct: u8,
    default_sound_volume_pct: u8,
    default_night_mode_sound_volume_pct: u8,
}

impl ConfigHandle {
    /// Load and validate the config, migrating a legacy schema *in memory* if needed.
    /// The on-disk file is **not** rewritten here —
    /// a migrated config is persisted only on the first genuine change (see [`Self::save`]).
    async fn load_and_validate(
        path: &Path,
    ) -> Result<(Config, bool, IndexMap<AccountId, Account>)> {
        let loaded = crate::config_migration::load_any_version(path).await?;
        if let Some(report) = loaded.report() {
            info!(
                scenes = report.scenes,
                dropped_scenes = report.dropped_scenes,
                deactivated_scenes = report.deactivated_scenes,
                translated_widgets = report.translated_widgets,
                dropped_widgets = report.dropped_widgets,
                "migrated legacy config in memory; will persist on next config change",
            );
        }
        let migrated = loaded.was_migrated();
        let (config, accounts) = loaded.into_parts();
        config
            .validate()
            .context("loaded config failed validation")?;
        Ok((config, migrated, accounts))
    }

    /// Fallback when [`Self::load_and_validate`] fails: the on-disk file
    /// is unreadable, corrupt, or declares a schema this BMC application
    /// cannot read (e.g. after an unsupported downgrade). Back it up next
    /// to the original and replace it with a platform default. Downgrades
    /// are not supported, so a config newer than this application
    /// understands is treated like any other unreadable file rather than
    /// preserved in place.
    async fn recover_from_failed_load(path: &Path, product: bmc_platform::Product) -> Config {
        warn!("Replacing unreadable config with default config");
        if path.exists() {
            let backup_path = path.with_extension("json.bcp");
            if let Err(err) = fs::copy(path, &backup_path).await {
                warn!(
                    "Failed to back up config to {}: {err:#}",
                    backup_path.display()
                );
            } else {
                warn!("Backed up broken config to {}", backup_path.display());
            }
        }

        let mut default_config = Config::platform_default(product);
        if let Err(err) = default_config.save(path).await {
            warn!(?err, "Failed to save default config");
        }
        default_config
    }

    pub async fn init(
        path: PathBuf,
        default_brightness_pct: u8,
        default_night_mode_brightness_pct: u8,
        default_sound_volume_pct: u8,
        default_night_mode_sound_volume_pct: u8,
        product: bmc_platform::Product,
    ) -> (Self, IndexMap<AccountId, Account>) {
        // On first boot after an upgrade this migrates a legacy config *in memory*
        // (it is persisted on the first genuine change, not here) and,
        // for an already-current config, is a cheap no-op. See `crate::config_migration`.
        let (config, migrated, extracted) = match Self::load_and_validate(&path).await {
            Ok(parts) => parts,
            Err(err) => {
                warn!(?err, "Failed to load or migrate config");
                (
                    Self::recover_from_failed_load(&path, product).await,
                    false,
                    IndexMap::new(),
                )
            }
        };

        let handle = Self {
            path,
            config,
            config_notify: ConfigNotify::new(),
            migrated,
            localization_dirty: false,
            night_mode_schedule_dirty: false,
            led_settings_dirty: false,
            brightness_settings_dirty: false,
            sound_settings_dirty: false,
            screen_off_timeout_dirty: false,
            scenes_dirty: false,
            default_brightness_pct,
            default_night_mode_brightness_pct,
            default_sound_volume_pct,
            default_night_mode_sound_volume_pct,
        };
        (handle, extracted)
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

    pub fn subscribe_sound_settings_change(&self) -> broadcast::Receiver<()> {
        self.config_notify.subscribe_sound_settings_change()
    }

    pub fn subscribe_screen_off_timeout_change(&self) -> broadcast::Receiver<Option<u32>> {
        self.config_notify.subscribe_screen_off_timeout_change()
    }

    pub fn subscribe_scenes_change(&self) -> broadcast::Receiver<WidgetSceneMap> {
        self.config_notify.subscribe_scenes_change()
    }

    pub fn scenes_mut(&mut self) -> &mut IndexMap<SceneId, Scene> {
        self.scenes_dirty = true;
        &mut self.config.scenes
    }

    pub fn widget_scene_map(&self) -> WidgetSceneMap {
        self.config.widget_scene_map()
    }

    pub async fn save(&mut self) -> Result<()> {
        if self.scenes_dirty {
            self.config_notify
                .scenes_changed(self.config.widget_scene_map());
            self.scenes_dirty = false;
        }

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
        if self.sound_settings_dirty {
            self.config_notify.sound_settings_changed();
            self.sound_settings_dirty = false;
        }
        if self.screen_off_timeout_dirty {
            let timeout = self.night_mode().screen_off_timeout_secs;
            self.config_notify.screen_off_timeout_changed(timeout);
            self.screen_off_timeout_dirty = false;
        }

        // First persist after an in-memory migration commits the upgrade:
        // keep one timestamped backup of the pre-migration file, then fall
        // through to the normal write. Later saves take the plain path.
        if self.migrated {
            crate::config_migration::backup_existing(&self.path).await?;
            self.migrated = false;
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
        self.sound_settings_dirty = true;
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
        self.sound_settings_dirty = true;
    }

    pub fn sound_volume_pct(&self) -> u8 {
        self.sound_volume_pct
            .unwrap_or(self.default_sound_volume_pct)
    }

    pub(crate) fn alarms(&self) -> Vec<AlarmData> {
        self.alarms.clone().unwrap_or_default()
    }

    pub(crate) fn add_alarm(&mut self, alarm: AlarmData) {
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

    pub(crate) fn set_alarm(&mut self, alarm: AlarmData) {
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
pub(crate) struct NightModeConfigData {
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
    fn bmc100_platform_default_constructs_without_panic() {
        let _ = Config::platform_default(bmc_platform::Product::Bmc100);
    }

    #[test]
    fn validate_accepts_default_scene_cycling() {
        let config = Config::platform_default(bmc_platform::Product::Bmc100);
        config
            .validate()
            .expect("BUG: default scene cycling must validate");
    }

    #[test]
    fn validate_rejects_too_short_automatic_cycling_default_duration() {
        let mut config = Config::platform_default(bmc_platform::Product::Bmc100);
        config.set_scene_cycling(SceneCycling {
            automatic_cycling_default_duration: std::time::Duration::ZERO,
            ..SceneCycling::default()
        });
        assert!(
            config.validate().is_err(),
            "a sub-minimum global cycle duration must be rejected on load"
        );
    }

    /// Tempfile-backed `ConfigHandle` for notification tests.
    async fn fresh_handle() -> (tempfile::TempDir, ConfigHandle) {
        let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
        let path = tmp.path().join("bmc-config.json");
        let (handle, _) =
            ConfigHandle::init(path, 50, 50, 50, 50, bmc_platform::Product::Bmc100).await;
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

    #[tokio::test]
    async fn save_emits_scenes_snapshot_when_dirty() {
        use tempfile::tempdir;

        let dir = tempdir().expect("BUG: tempdir");
        let path = dir.path().join("config.json");
        let (mut handle, _) =
            ConfigHandle::init(path, 80, 30, 80, 30, bmc_platform::Product::Bmc100).await;

        let mut rx = handle.subscribe_scenes_change();

        handle.scenes_mut();
        let expected = handle.widget_scene_map();
        handle.save().await.expect("BUG: save must succeed");

        let snapshot = rx
            .recv()
            .await
            .expect("BUG: subscriber must receive snapshot");
        assert_eq!(snapshot, expected);
    }

    #[tokio::test]
    async fn save_does_not_emit_scenes_snapshot_when_clean() {
        use tempfile::tempdir;

        let dir = tempdir().expect("BUG: tempdir");
        let path = dir.path().join("config.json");
        let (mut handle, _) =
            ConfigHandle::init(path, 80, 30, 80, 30, bmc_platform::Product::Bmc100).await;

        let mut rx = handle.subscribe_scenes_change();
        handle.save().await.expect("BUG: save must succeed");

        assert!(
            rx.try_recv().is_err(),
            "no snapshot must be sent when scenes_dirty is false"
        );
    }

    #[tokio::test]
    async fn save_notifies_sound_settings_when_volume_dirty() {
        let (_tmp, mut handle) = fresh_handle().await;
        let mut rx = handle.subscribe_sound_settings_change();

        handle.set_sound_volume(35);
        handle.save().await.expect("BUG: save must succeed");

        assert!(rx.try_recv().is_ok(), "day volume write must notify");

        handle.set_night_mode_sound_volume(15);
        handle.save().await.expect("BUG: save must succeed");

        assert!(rx.try_recv().is_ok(), "night volume write must notify");
    }

    #[tokio::test]
    async fn save_does_not_notify_sound_settings_when_clean() {
        let (_tmp, mut handle) = fresh_handle().await;
        let mut rx = handle.subscribe_sound_settings_change();

        handle.save().await.expect("BUG: save must succeed");

        assert!(
            rx.try_recv().is_err(),
            "no notification must be sent when sound settings are untouched"
        );
    }

    /// Count `config.json.backup.<ts>` siblings in `dir`.
    async fn backup_count(dir: &std::path::Path) -> usize {
        let mut count = 0;
        let mut entries = fs::read_dir(dir).await.expect("BUG: readdir");
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry
                .file_name()
                .to_string_lossy()
                .contains("config.json.backup.")
            {
                count += 1;
            }
        }
        count
    }

    /// A legacy (`version: 0`) config on disk is migrated *in memory*
    /// when `ConfigHandle::init` loads it — the on-disk file is left at
    /// its old version. The upgrade is committed to disk only on the
    /// first genuine change, which also leaves one timestamped backup of
    /// the pre-migration file.
    #[tokio::test]
    async fn init_migrates_legacy_config_in_memory_and_commits_on_save() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("config.json");
        let legacy = r#"{
            "scenes": [
                {
                    "id": "a418d38d-a506-489d-9627-0c7909374ef1",
                    "enabled": true,
                    "kind": "fullscreen",
                    "widgets": [
                        {
                            "id": "3c32f8c7-e678-466d-a331-39b5c8f89153",
                            "row": 0, "col": 0, "size": "full", "kind": "clock",
                            "params": { "clock_style": "digital" }
                        }
                    ]
                }
            ],
            "accounts": [],
            "brightness_pct": 30,
            "alarms": [{
                "id": "wake-up",
                "enabled": true,
                "name": "Wake up",
                "time": "07:00:00",
                "repeat": []
            }]
        }"#;
        fs::write(&path, legacy)
            .await
            .expect("BUG: seed legacy file");

        let (mut handle, _) =
            ConfigHandle::init(path.clone(), 50, 50, 50, 50, bmc_platform::Product::Bmc100).await;
        // Migrated in memory: the running config is the current schema.
        assert_eq!(handle.config.version, CONFIG_VERSION);
        assert_eq!(handle.config.scenes().len(), 1);

        // But the on-disk file is untouched — still the legacy shape with
        // no version field, and no backup written yet.
        let on_disk = fs::read_to_string(&path).await.expect("BUG: read");
        let v: serde_json::Value = serde_json::from_str(&on_disk).expect("BUG: valid JSON");
        assert!(
            v.get("version").is_none(),
            "boot must not rewrite the on-disk config"
        );
        assert_eq!(
            backup_count(dir.path()).await,
            0,
            "no backup before first save"
        );

        // A genuine change commits the upgrade: the file becomes the
        // current schema, settings survive, and exactly one timestamped
        // backup of the original is left.
        handle.set_brightness(80);
        handle.save().await.expect("BUG: save");
        let on_disk = fs::read_to_string(&path).await.expect("BUG: read migrated");
        let v: serde_json::Value = serde_json::from_str(&on_disk).expect("BUG: valid JSON");
        assert_eq!(v["version"], CONFIG_VERSION);
        assert_eq!(v["brightness_pct"], 80);
        assert_eq!(v["alarms"][0]["time"], "07:00:00");
        assert_eq!(
            backup_count(dir.path()).await,
            1,
            "first save leaves one backup"
        );

        // A later save must not create another backup.
        handle.set_brightness(70);
        handle.save().await.expect("BUG: save again");
        assert_eq!(
            backup_count(dir.path()).await,
            1,
            "later saves do not back up"
        );
    }

    /// Downgrades are not supported: a config whose version is newer
    /// than this BMC application understands is treated like any other
    /// unreadable file — backed up to `<name>.json.bcp` and replaced
    /// with a platform default, rather than preserved in place.
    #[tokio::test]
    async fn init_backs_up_and_replaces_unreadable_newer_config() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let path = dir.path().join("config.json");
        let newer = format!(r#"{{"version":{},"scenes":[]}}"#, CONFIG_VERSION + 1);
        fs::write(&path, &newer)
            .await
            .expect("BUG: seed newer file");

        let (handle, _) =
            ConfigHandle::init(path.clone(), 50, 50, 50, 50, bmc_platform::Product::Bmc100).await;
        assert_eq!(
            handle.config.version, CONFIG_VERSION,
            "runs on a current-schema default"
        );

        // Canonical path now holds a default; the unreadable newer config
        // survives only in the `.bcp` backup.
        let on_disk = fs::read_to_string(&path).await.expect("BUG: read file");
        let v: serde_json::Value = serde_json::from_str(&on_disk).expect("BUG: valid JSON");
        assert_eq!(v["version"], CONFIG_VERSION);
        let bcp = path.with_extension("json.bcp");
        assert_eq!(
            fs::read_to_string(&bcp).await.expect("BUG: read bcp"),
            newer,
            "the newer config is preserved in the .bcp backup"
        );
    }

    /// On a factory-fresh device the config directory (`/etc/bmc/`)
    /// does not exist yet and there is no file to migrate. Boot must
    /// still write a usable default config, creating the parent
    /// directory on the way — otherwise every later save fails with
    /// ENOENT and settings never persist across reboots.
    #[tokio::test]
    async fn init_on_fresh_install_creates_dir_and_persists_default() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        // Parent directory intentionally absent, like `/etc/bmc/` on a
        // fresh flash.
        let path = dir.path().join("bmc").join("config.json");
        assert!(!path.exists(), "precondition: config must not exist yet");

        let (handle, _) =
            ConfigHandle::init(path.clone(), 50, 50, 50, 50, bmc_platform::Product::Bmc100).await;
        assert_eq!(handle.config.version, CONFIG_VERSION);

        let on_disk = fs::read_to_string(&path)
            .await
            .expect("BUG: default config must be written to a fresh path");
        let v: serde_json::Value = serde_json::from_str(&on_disk).expect("BUG: valid JSON");
        assert_eq!(v["version"], CONFIG_VERSION);
    }
}

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

use crate::config::ConfigHandle;
use crate::i18n::I18nMarker;
use crate::sound::Sounds;
use crate::system_manager::SystemManager;

use crate::backlight::DisplayBacklightDriver;
use bmc_grpc::web::{
    self, BootSoundSettingsResponse, GeneralSettingsDataResponse, LedSettingsResponse,
    ListSoundsResponse, PlaySoundRequest, SetDateFormatRequest, SetFirstDayOfWeekRequest,
    SetLanguageRequest, SetNumberFormatRequest, SetTemperatureUnitRequest, SetTimeFormatRequest,
    SetUnitSystemRequest, SoundInfo, SoundVolume,
};
use bmc_grpc::web::{
    BrightnessInfo, ConfigurationService as GrpcConfigurationService, DisplaySettingsResponse,
    SoundVolumeSettingsResponse, TimeInterval,
};
use bmc_intl::{Intl, Translations, format_message};
use bmc_shared_time::time::{DateFormat, TimeSystem};
use bmc_shared_utils::number_format::NumberFormat;
use bmc_shared_utils::temperature::TemperatureUnit;
use bmc_shared_utils::unit_system::UnitSystem;
use buffa_types::google::protobuf::{BoolValue, Empty, UInt32Value};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use std::str::FromStr;
use std::sync::Arc;
use strum::IntoEnumIterator;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

use super::alarm::{map_weekday_from_proto, map_weekday_to_proto};
use super::initial_setup::{
    try_from_date_time, try_from_temperature_unit, try_from_time_format, try_from_unit_system,
};
use super::intl::intl;
use super::language::{listed_tag, save_language};
use super::shared::{
    bad_request, invalid_field, naive_time_to_hhmm, parse_hhmm_to_naive_time,
    try_from_number_format,
};
use crate::sound::SoundController;

const API_BRIGHTNESS_MIN: u32 = crate::backlight::MIN_BRIGHTNESS_PCT as u32;
const API_BRIGHTNESS_MAX: u32 = 100;
const API_BRIGHTNESS_STEP: u32 = 5;

const API_SOUND_VOLUME_MIN: u32 = 0;
const API_SOUND_VOLUME_MAX: u32 = 100;
const API_SOUND_VOLUME_STEP: u32 = 1;

pub(crate) struct ConfigurationService<T: DisplayBacklightDriver> {
    system_manager: SystemManager<T>,
    sound_controller: SoundController,
    config_handle: Arc<RwLock<ConfigHandle>>,
    i18n: I18nMarker,
}

impl<T: DisplayBacklightDriver> ConfigurationService<T> {
    pub(crate) fn new(
        system_manager: SystemManager<T>,
        sound_controller: SoundController,
        config_handle: Arc<RwLock<ConfigHandle>>,
        i18n: I18nMarker,
    ) -> Self {
        Self {
            system_manager,
            sound_controller,
            config_handle,
            i18n,
        }
    }
}

impl<T: DisplayBacklightDriver> GrpcConfigurationService for ConfigurationService<T> {
    async fn get_display_settings(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<DisplaySettingsResponse> {
        let display_settings = self.system_manager.display_settings().await;

        Response::ok(DisplaySettingsResponse {
            brightness: BrightnessInfo {
                value: u32::from(display_settings.brightness_pct),
                min: API_BRIGHTNESS_MIN,
                max: API_BRIGHTNESS_MAX,
                step: API_BRIGHTNESS_STEP,
            }
            .into(),

            brightness_nightmode: BrightnessInfo {
                value: u32::from(display_settings.night_mode_config.brightness_pct),
                min: API_BRIGHTNESS_MIN,
                max: API_BRIGHTNESS_MAX,
                step: API_BRIGHTNESS_STEP,
            }
            .into(),
            nightmode_enabled: display_settings.night_mode_config.enabled,
            nightmode_interval: TimeInterval {
                from: naive_time_to_hhmm(display_settings.night_mode_config.from),
                to: naive_time_to_hhmm(display_settings.night_mode_config.to),
            }
            .into(),
            nightmode_screen_off_timeout_secs: display_settings
                .night_mode_config
                .screen_off_timeout_secs
                .map(|secs| UInt32Value {
                    value: secs,
                    ..Default::default()
                })
                .into(),
        })
    }

    async fn set_brightness(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UInt32Value>,
    ) -> ServiceResult<Empty> {
        let value = clamp_brightness(request.value);

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_brightness(value as u8)
            .await
            .map_err(|e| {
                warn!("Cannot set display brightness: {}", e);
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to set the display brightness!",
                    description: "Error when the device could not apply the screen brightness setting",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn set_brightness_nightmode(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UInt32Value>,
    ) -> ServiceResult<Empty> {
        let value = clamp_brightness(request.value);

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_night_mode_brightness(value as u8)
            .await
            .map_err(|e| {
                warn!("Cannot set night mode brightness, error: {}", e);
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to set the night mode brightness!",
                    description: "Error when the device could not apply the screen brightness setting for night mode",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn set_nightmode_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BoolValue>,
    ) -> ServiceResult<Empty> {
        self.system_manager
            .set_night_mode_enabled(request.value)
            .await
            .map_err(|e| {
                error!("Failed to enable/disable night mode, error {}", e);
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to switch night mode!",
                    description: "Error when the device could not apply the night mode switch",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn set_nightmode_interval(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, TimeInterval>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let from = parse_hhmm_to_naive_time(&intl, request.from)?;
        let to = parse_hhmm_to_naive_time(&intl, request.to)?;

        self.system_manager
            .set_night_mode_interval(from, to)
            .await
            .map_err(|e| {
                error!("Failed to set night mode interval, error: {}", e);
                ConnectError::internal(format_message!(
                    intl,
                    default_message: "Failed to set the night mode time interval!",
                    description: "Error when the device could not apply the hours night mode runs between",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn get_sound_volume_settings(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<SoundVolumeSettingsResponse> {
        let sound_settings = self.system_manager.sound_settings().await;
        Response::ok(SoundVolumeSettingsResponse {
            volume: SoundVolume {
                value: u32::from(sound_settings.volume),
                min: API_SOUND_VOLUME_MIN,
                max: API_SOUND_VOLUME_MAX,
                step: API_SOUND_VOLUME_STEP,
            }
            .into(),
            volume_nightmode: SoundVolume {
                value: u32::from(sound_settings.volume_night_mode),
                min: API_SOUND_VOLUME_MIN,
                max: API_SOUND_VOLUME_MAX,
                step: API_SOUND_VOLUME_STEP,
            }
            .into(),
        })
    }

    async fn set_sound_volume(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UInt32Value>,
    ) -> ServiceResult<Empty> {
        let value = request.value;
        let intl = intl(&ctx);

        validate_sound_volume(&intl, value)?;

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_sound_volume(value as u8)
            .await
            .map_err(|e| {
                warn!("Failed to set sound volume, error: {}", e);
                ConnectError::internal(format_message!(
                    intl,
                    default_message: "Failed to set the sound volume!",
                    description: "Error when the device could not apply the sound volume setting",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn set_sound_volume_nightmode(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UInt32Value>,
    ) -> ServiceResult<Empty> {
        let value = request.value;
        let intl = intl(&ctx);

        validate_sound_volume(&intl, value)?;

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_sound_volume_night_mode(value as u8)
            .await
            .map_err(|e| {
                warn!("Failed to set sound volume for night mode, error: {}", e);
                ConnectError::internal(format_message!(
                    intl,
                    default_message: "Failed to set the night mode sound volume!",
                    description: "Error when the device could not apply the sound volume setting for night mode",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn list_sounds(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListSoundsResponse> {
        let sounds: Vec<SoundInfo> = Sounds::iter().map(Into::into).collect();

        Response::ok(ListSoundsResponse { sounds })
    }

    async fn play_sound(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PlaySoundRequest>,
    ) -> ServiceResult<Empty> {
        let sound = Sounds::from_str(request.sound_id).map_err(|e| {
            warn!("Failed to parse sound, error {e}");
            ConnectError::invalid_argument(format_message!(
                intl(&ctx),
                default_message: "Invalid sound!",
                description: "Error when playing a sound the device does not have",
            ))
        })?;

        let token = CancellationToken::new();
        // NOTE: _drop_guard works here as an automatic cancellation of the sound playing.
        // If user cancels the operation e.g. cancels gRPC request, _drop_guard is dropped which cancels the token
        // which cancels the process of sound play.
        let _drop_guard = token.clone().drop_guard();

        let sound_controller = self.sound_controller.clone();

        // NOTE: Spawn `Audio::play` to avoid blocking cancellation;
        // running it inline would delay dropping the token and prevent cancellation.
        let played = tokio::spawn(async move {
            sound_controller
                .play_sound(sound, token)
                .await
                .inspect_err(|e| error!("Failed to play sound, error {e}"))
        })
        .await;
        if !matches!(played, Ok(Ok(()))) {
            return Err(ConnectError::internal(format_message!(
                intl(&ctx),
                default_message: "Failed to play the sound!",
                description: "Error when the device could not play the sound chosen to preview",
            )));
        }

        Response::ok(Empty::default())
    }

    async fn get_general_settings_data(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<GeneralSettingsDataResponse> {
        let (localization, data_collection) = {
            let config = self.config_handle.read().await;
            let localization = config.localization_config();

            let data_collection = config.data_collection();
            (localization, data_collection)
        };

        Response::ok(GeneralSettingsDataResponse {
            data_collection: BoolValue {
                value: data_collection,
                ..Default::default()
            }
            .into(),
            time_format: map_time_system_to_proto(localization.time_system).into(),
            date_format: map_date_format_to_proto(localization.date_format).into(),
            number_format: map_number_format_to_proto(localization.number_format).into(),
            first_day_of_week: map_weekday_to_proto(localization.first_day_of_week).into(),
            temperature_unit: map_temperature_unit_to_proto(localization.temperature_unit).into(),
            unit_system: map_unit_system_to_proto(localization.unit_system).into(),
            // Absent without the marker, as the BMC application sent it before the setting.
            language: if self.i18n.is_set() {
                listed_tag(&Translations::shared(), localization.language())
            } else {
                None
            },
            show_seconds_status_bar: BoolValue {
                value: localization.show_seconds_in_status_bar,
                ..Default::default()
            }
            .into(),
        })
    }

    async fn set_time_format(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetTimeFormatRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let time_system = try_from_time_format(&intl, request.time_format)
            .map_err(|violation| bad_request(&intl, vec![violation]))?;

        let mut config = self.config_handle.write().await;

        config.set_time_system(time_system);

        config.save().await.map_err(|e| {
            error!("Failed to save time_system, error {e}");
            ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to save the time format!",
                description: "Error when the device could not store the 12 or 24 hour clock setting",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn set_number_format(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetNumberFormatRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let number_format = try_from_number_format(&intl, request.number_format)
            .map_err(|violation| bad_request(&intl, vec![violation]))?;

        let mut config = self.config_handle.write().await;
        config.set_number_format(number_format);

        config.save().await.map_err(|e| {
            error!("Failed to save number_format, error {e}");
            ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to save the number format!",
                description: "Error when the device could not store how numbers are grouped and punctuated",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn set_date_format(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetDateFormatRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let date_format = try_from_date_time(&intl, request.date_format)
            .map_err(|violation| bad_request(&intl, vec![violation]))?;

        let mut config = self.config_handle.write().await;
        config.set_date_format(date_format);

        config.save().await.map_err(|e| {
            error!("Failed to save date_format, error {e}");
            ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to save the date format!",
                description: "Error when the device could not store how dates are written",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn set_data_collection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BoolValue>,
    ) -> ServiceResult<Empty> {
        let mut config = self.config_handle.write().await;
        config.set_data_collection(request.value);

        config.save().await.map_err(|e| {
            error!("Failed to save data_collection, error {e}");
            ConnectError::internal(format_message!(
                intl(&ctx),
                default_message: "Failed to save the data collection setting!",
                description: "Error when the device could not store whether it may collect usage data",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn set_first_day_of_week(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetFirstDayOfWeekRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let weekday = request
            .first_day_of_week
            .as_known()
            .and_then(map_weekday_from_proto)
            .ok_or_else(|| {
                let description = format_message!(
                    intl,
                    default_message: "Choose the first day of the week",
                    description: "Error under the first day of the week field when nothing is chosen",
                );
                invalid_field(&intl, "first_day_of_week", description)
            })?;

        let mut config = self.config_handle.write().await;
        config.set_first_day_of_week(weekday);

        config.save().await.map_err(|e| {
            error!("Failed to save first_day_of_week, error {e}");
            ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to save the first day of the week!",
                description: "Error when the device could not store which day its weeks start on",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn set_temperature_unit(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetTemperatureUnitRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let temperature_unit = try_from_temperature_unit(&intl, request.temperature_unit)
            .map_err(|violation| bad_request(&intl, vec![violation]))?;

        let mut config = self.config_handle.write().await;
        config.set_temperature_unit(temperature_unit);

        config.save().await.map_err(|e| {
            error!("Failed to save temperature_unit, error {e}");
            ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to save the temperature unit!",
                description: "Error when the device could not store whether it shows Celsius or Fahrenheit",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn set_unit_system(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetUnitSystemRequest>,
    ) -> ServiceResult<Empty> {
        let intl = intl(&ctx);
        let unit_system = try_from_unit_system(&intl, request.unit_system)
            .map_err(|violation| bad_request(&intl, vec![violation]))?;

        let mut config = self.config_handle.write().await;
        config.set_unit_system(unit_system);

        config.save().await.map_err(|e| {
            error!("Failed to save unit_system, error {e}");
            ConnectError::internal(format_message!(
                intl,
                default_message: "Failed to save the unit system!",
                description: "Error when the device could not store whether it uses metric or imperial units",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn set_language(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetLanguageRequest>,
    ) -> ServiceResult<Empty> {
        save_language(&intl(&ctx), &self.config_handle, request.language).await?;
        Response::ok(Empty::default())
    }

    async fn show_seconds_in_status_bar(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BoolValue>,
    ) -> ServiceResult<Empty> {
        let mut config = self.config_handle.write().await;
        config.show_seconds_in_status_bar(request.value);

        config.save().await.map_err(|e| {
            error!("Failed to show/hide seconds in status bar, error {e}");
            ConnectError::internal(format_message!(
                intl(&ctx),
                default_message: "Failed to show or hide the seconds in the status bar!",
                description: "Error when the device could not store whether its status bar clock shows seconds",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn get_led_settings(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<LedSettingsResponse> {
        let led_settings = self.system_manager.led_settings().await;

        Response::ok(LedSettingsResponse {
            led_enabled: led_settings.led_enabled,
            led_enabled_nightmode: led_settings.led_enabled_night_mode,
        })
    }

    async fn set_led_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BoolValue>,
    ) -> ServiceResult<Empty> {
        self.system_manager
            .set_led_enabled(request.value)
            .await
            .map_err(|e| {
                error!("Failed to set led_enabled, error {e}");
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to save the LED notifications setting!",
                    description: "Error when the device could not store whether its LEDs show notifications",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn set_led_enabled_nightmode(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BoolValue>,
    ) -> ServiceResult<Empty> {
        self.system_manager
            .set_led_enabled_night_mode(request.value)
            .await
            .map_err(|e| {
                error!("Failed to set led_enabled_nightmode, error {e}");
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to save the night mode LED notifications setting!",
                    description: "Error when the device could not store whether its LEDs show notifications during night mode",
                ))
            })?;

        Response::ok(Empty::default())
    }

    async fn get_boot_sound_settings(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<BootSoundSettingsResponse> {
        let boot_sound_enabled = self.config_handle.read().await.boot_sound_enabled();

        Response::ok(BootSoundSettingsResponse { boot_sound_enabled })
    }

    async fn set_boot_sound_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BoolValue>,
    ) -> ServiceResult<Empty> {
        let mut config = self.config_handle.write().await;
        let enabled = request.value;

        if config.boot_sound_enabled() == enabled {
            return Response::ok(Empty::default());
        }

        config.set_boot_sound_enabled(enabled);

        config.save().await.map_err(|e| {
            error!("Failed to save boot_sound_enabled, error {e}");
            ConnectError::internal(format_message!(
                intl(&ctx),
                default_message: "Failed to save the boot sound setting!",
                description: "Error when the device could not store whether it plays a sound when it starts",
            ))
        })?;

        Response::ok(Empty::default())
    }

    async fn set_nightmode_screen_off_timeout(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UInt32Value>,
    ) -> ServiceResult<Empty> {
        let value = request.value;

        let timeout = if value == 0 { None } else { Some(value) };

        self.system_manager
            .set_night_mode_screen_off_timeout(timeout)
            .await
            .map_err(|e| {
                error!("Failed to set night mode screen off timeout, error: {e}");
                ConnectError::internal(format_message!(
                    intl(&ctx),
                    default_message: "Failed to set the night mode screen-off timeout!",
                    description: "Error when the device could not apply how long night mode waits before turning the screen off",
                ))
            })?;

        Response::ok(Empty::default())
    }
}

fn clamp_brightness(value: u32) -> u32 {
    value.clamp(API_BRIGHTNESS_MIN, API_BRIGHTNESS_MAX)
}

fn validate_sound_volume(intl: &Intl, value: u32) -> Result<(), ConnectError> {
    if value > API_SOUND_VOLUME_MAX {
        return Err(ConnectError::invalid_argument(format_message!(
            intl,
            default_message: "The sound volume must be between {min} and {max}.",
            description: "Error when a sound volume is out of range; min and max are whole numbers",
            values: {
                min: u64::from(API_SOUND_VOLUME_MIN),
                max: u64::from(API_SOUND_VOLUME_MAX),
            },
        )));
    }
    Ok(())
}

impl From<Sounds> for SoundInfo {
    fn from(value: Sounds) -> Self {
        Self {
            id: value.to_string(),
            name: value.name().to_owned(),
        }
    }
}

fn map_time_system_to_proto(value: TimeSystem) -> web::TimeFormat {
    match value {
        TimeSystem::Hour12 => web::TimeFormat::TimeFormat12Hour,
        TimeSystem::Hour24 => web::TimeFormat::TimeFormat24Hour,
    }
}

fn map_date_format_to_proto(value: DateFormat) -> web::DateFormat {
    match value {
        DateFormat::DdMmYyyyDot => web::DateFormat::DdMmYyyyDot,
        DateFormat::DdMmYyyySlash => web::DateFormat::DdMmYyyySlash,
        DateFormat::DMYyyySlash => web::DateFormat::DMYyyySlash,
        DateFormat::MDYyyySlash => web::DateFormat::MDYyyySlash,
        DateFormat::DdMmYyyyDash => web::DateFormat::DdMmYyyyDash,
        DateFormat::YyyyMDSlash => web::DateFormat::YyyyMDSlash,
        DateFormat::YyyyMmDdDot => web::DateFormat::YyyyMmDdDot,
        DateFormat::YyyyMmDdDash => web::DateFormat::YyyyMmDdDash,
    }
}

fn map_number_format_to_proto(value: NumberFormat) -> web::NumberFormat {
    match value {
        NumberFormat::SpaceGroupCommaDecimal => web::NumberFormat::SpaceGroupCommaDecimal,
        NumberFormat::CommaGroupDotDecimal => web::NumberFormat::CommaGroupDotDecimal,
        NumberFormat::DotGroupCommaDecimal => web::NumberFormat::DotGroupCommaDecimal,
        NumberFormat::SpaceGroupDotDecimal => web::NumberFormat::SpaceGroupDotDecimal,
    }
}

fn map_temperature_unit_to_proto(value: TemperatureUnit) -> web::TemperatureUnit {
    match value {
        TemperatureUnit::Celsius => web::TemperatureUnit::Celsius,
        TemperatureUnit::Fahrenheit => web::TemperatureUnit::Fahrenheit,
    }
}

fn map_unit_system_to_proto(value: UnitSystem) -> web::UnitSystem {
    match value {
        UnitSystem::Metric => web::UnitSystem::Metric,
        UnitSystem::Imperial => web::UnitSystem::Imperial,
    }
}

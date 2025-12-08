// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::{ConfigHandle, UnitSystem};
use crate::sound::Sounds;
use crate::system_manager::SystemManager;

use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_grpc::web::{
    self, BootSoundSettingsResponse, GeneralSettingsDataResponse, LedSettingsResponse,
    ListSoundsResponse, PlaySoundRequest, SetDateFormatRequest, SetFirstDayOfWeekRequest,
    SetNumberFormatRequest, SetTemperatureUnitRequest, SetTimeFormatRequest, SetUnitSystemRequest,
    SoundInfo, SoundVolume,
};
use bmc_grpc::web::{
    BrightnessInfo, DisplaySettingsResponse, SoundVolumeSettingsResponse, TimeInterval,
    configuration_service_server::ConfigurationService as GrpcConfigurationService,
};
use bmc_shared_time::time::{DateFormat, TimeSystem};
use bmc_shared_utils::number_format::NumberFormat;
use bmc_shared_utils::temperature::TemperatureUnit;
use std::str::FromStr;
use std::sync::Arc;
use strum::IntoEnumIterator;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};
use tracing::{error, warn};

use super::alarm::{map_weekday_from_proto, map_weekday_to_proto};
use super::initial_setup::{try_from_date_time, try_from_time_format};
use super::shared::{naive_time_to_hhmm, parse_hhmm_to_naive_time, try_from_number_format};
use super::{GrpcError, SoundController};

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
}

impl<T: DisplayBacklightDriver> ConfigurationService<T> {
    pub(crate) fn new(
        system_manager: SystemManager<T>,
        sound_controller: SoundController,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) -> Self {
        Self {
            system_manager,
            sound_controller,
            config_handle,
        }
    }
}

#[async_trait::async_trait]

impl<T: DisplayBacklightDriver> GrpcConfigurationService for ConfigurationService<T> {
    async fn get_display_settings(
        &self,
        _request: Request<()>,
    ) -> Result<Response<DisplaySettingsResponse>, Status> {
        let display_settings = self.system_manager.display_settings().await;

        Ok(Response::new(DisplaySettingsResponse {
            brightness: Some(BrightnessInfo {
                value: u32::from(display_settings.brightness_pct),
                min: API_BRIGHTNESS_MIN,
                max: API_BRIGHTNESS_MAX,
                step: API_BRIGHTNESS_STEP,
            }),

            brightness_nightmode: Some(BrightnessInfo {
                value: u32::from(display_settings.night_mode_config.brightness_pct),
                min: API_BRIGHTNESS_MIN,
                max: API_BRIGHTNESS_MAX,
                step: API_BRIGHTNESS_STEP,
            }),
            nightmode_enabled: display_settings.night_mode_config.enabled,
            nightmode_interval: Some(TimeInterval {
                from: naive_time_to_hhmm(display_settings.night_mode_config.from),
                to: naive_time_to_hhmm(display_settings.night_mode_config.to),
            }),
            nightmode_screen_off_timeout_secs: display_settings
                .night_mode_config
                .screen_off_timeout_secs,
        }))
    }

    async fn set_brightness(&self, request: Request<u32>) -> Result<Response<()>, Status> {
        let value = clamp_brightness(request.into_inner());

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_brightness(value as u8)
            .await
            .map_err(|e| {
                warn!("Cannot set display brightness: {}", e);
                Status::internal("Failed to set display brightness")
            })?;

        Ok(Response::new(()))
    }

    async fn set_brightness_nightmode(
        &self,
        request: Request<u32>,
    ) -> Result<Response<()>, Status> {
        let value = clamp_brightness(request.into_inner());

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_night_mode_brightness(value as u8)
            .await
            .map_err(|e| {
                warn!("Cannot set night mode brightness, error: {}", e);
                Status::internal("Failed to set night mode brightness")
            })?;

        Ok(Response::new(()))
    }

    async fn set_nightmode_enabled(&self, request: Request<bool>) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        self.system_manager
            .set_night_mode_enabled(value)
            .await
            .map_err(|e| {
                error!("Failed to enable/disable night mode, error {}", e);
                Status::internal("Failed to enable/disable night mode")
            })?;

        Ok(Response::new(()))
    }

    async fn set_nightmode_interval(
        &self,
        request: Request<TimeInterval>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();

        let from = parse_hhmm_to_naive_time(&request.from)?;
        let to = parse_hhmm_to_naive_time(&request.to)?;

        self.system_manager
            .set_night_mode_interval(from, to)
            .await
            .map_err(|e| {
                error!("Failed to set night mode interval, error: {}", e);
                Status::internal("Failed to set night mode interval")
            })?;

        Ok(tonic::Response::new(()))
    }

    async fn get_sound_volume_settings(
        &self,
        _request: Request<()>,
    ) -> Result<Response<SoundVolumeSettingsResponse>, Status> {
        let sound_settings = self.system_manager.sound_settings().await;
        Ok(Response::new(SoundVolumeSettingsResponse {
            volume: Some(SoundVolume {
                value: u32::from(sound_settings.volume),
                min: API_SOUND_VOLUME_MIN,
                max: API_SOUND_VOLUME_MAX,
                step: API_SOUND_VOLUME_STEP,
            }),
            volume_nightmode: Some(SoundVolume {
                value: u32::from(sound_settings.volume_night_mode),
                min: API_SOUND_VOLUME_MIN,
                max: API_SOUND_VOLUME_MAX,
                step: API_SOUND_VOLUME_STEP,
            }),
        }))
    }

    async fn set_sound_volume(&self, request: Request<u32>) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        validate_sound_volume(value)?;

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_sound_volume(value as u8)
            .await
            .map_err(|e| {
                warn!("Failed to set sound volume, error: {}", e);
                Status::internal("Failed to set sound volume")
            })?;

        Ok(Response::new(()))
    }

    async fn set_sound_volume_nightmode(
        &self,
        request: Request<u32>,
    ) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        validate_sound_volume(value)?;

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_sound_volume_night_mode(value as u8)
            .await
            .map_err(|e| {
                warn!("Failed to set sound volume for night mode, error: {}", e);
                Status::internal("Failed to set sound volume for night mode")
            })?;

        Ok(Response::new(()))
    }

    async fn list_sounds(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ListSoundsResponse>, Status> {
        let sounds: Vec<SoundInfo> = Sounds::iter().map(Into::into).collect();

        Ok(Response::new(ListSoundsResponse { sounds }))
    }

    async fn play_sound(&self, request: Request<PlaySoundRequest>) -> Result<Response<()>, Status> {
        let sound = Sounds::from_str(&request.into_inner().sound_id).map_err(|e| {
            warn!("Failed to parse sound, error {e}");
            Status::invalid_argument("Invalid sound_id")
        })?;

        let token = CancellationToken::new();
        // NOTE: _drop_guard works here as an automatic cancellation of the sound playing.
        // If user cancels the operation e.g. cancels gRPC request, _drop_guard is dropped which cancels the token
        // which cancels the process of sound play.
        let _drop_guard = token.clone().drop_guard();

        let sound_controller = self.sound_controller.clone();

        // NOTE: Spawn `Audio::play` to avoid blocking cancellation;
        // running it inline would delay dropping the token and prevent cancellation.
        tokio::spawn(async move {
            sound_controller
                .play_sound(sound, token)
                .await
                .map_err(|e| {
                    error!("Failed to play sound, error {e}");
                    Status::internal("Failed to play sound")
                })?;

            Ok::<(), Status>(())
        })
        .await
        .map_err(|_| Status::internal("Failed to play sound"))??;

        Ok(Response::new(()))
    }

    async fn get_general_settings_data(
        &self,
        _request: Request<()>,
    ) -> Result<Response<GeneralSettingsDataResponse>, Status> {
        let (localization, data_collection) = {
            let config = self.config_handle.read().await;
            let localization = config.localization_config();

            let data_collection = config.data_collection();
            (localization, data_collection)
        };

        Ok(Response::new(GeneralSettingsDataResponse {
            data_collection: Some(data_collection),
            time_format: map_time_system_to_proto(localization.time_system).into(),
            date_format: map_date_format_to_proto(localization.date_format).into(),
            number_format: map_number_format_to_proto(localization.number_format).into(),
            first_day_of_week: map_weekday_to_proto(localization.first_day_of_week).into(),
            temperature_unit: map_temperature_unit_to_proto(localization.temperature_unit).into(),
            unit_system: map_unit_system_to_proto(&localization.unit_system).into(),
            show_seconds_status_bar: Some(localization.show_seconds_in_status_bar),
        }))
    }

    async fn set_time_format(
        &self,
        request: Request<SetTimeFormatRequest>,
    ) -> Result<Response<()>, Status> {
        let time_system = try_from_time_format(request.into_inner().time_format()).map_err(
            |field_violation| {
                Status::with_error_details(
                    Code::InvalidArgument,
                    GrpcError::BadRequest.to_string(),
                    ErrorDetails::with_bad_request([field_violation]),
                )
            },
        )?;

        let mut config = self.config_handle.write().await;

        config.set_time_system(time_system);

        config.save().await.map_err(|e| {
            error!("Failed to save time_system, error {e}");
            Status::internal("Failed to save time_system")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn set_number_format(
        &self,
        request: Request<SetNumberFormatRequest>,
    ) -> Result<Response<()>, Status> {
        let number_format = try_from_number_format(request.into_inner().number_format()).map_err(
            |field_violation| {
                Status::with_error_details(
                    Code::InvalidArgument,
                    GrpcError::BadRequest.to_string(),
                    ErrorDetails::with_bad_request([field_violation]),
                )
            },
        )?;

        let mut config = self.config_handle.write().await;
        config.set_number_format(number_format);

        config.save().await.map_err(|e| {
            error!("Failed to save number_format, error {e}");
            Status::internal("Failed to save number_format")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn set_date_format(
        &self,
        request: Request<SetDateFormatRequest>,
    ) -> Result<Response<()>, Status> {
        let date_format =
            try_from_date_time(request.into_inner().date_format()).map_err(|field_violation| {
                Status::with_error_details(
                    Code::InvalidArgument,
                    GrpcError::BadRequest.to_string(),
                    ErrorDetails::with_bad_request([field_violation]),
                )
            })?;

        let mut config = self.config_handle.write().await;
        config.set_date_format(date_format);

        config.save().await.map_err(|e| {
            error!("Failed to save date_format, error {e}");
            Status::internal("Failed to save date_format")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn set_data_collection(&self, request: Request<bool>) -> Result<Response<()>, Status> {
        let mut config = self.config_handle.write().await;
        config.set_data_collection(request.into_inner());

        config.save().await.map_err(|e| {
            error!("Failed to save data_collection, error {e}");
            Status::internal("Failed to save data_collection")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn set_first_day_of_week(
        &self,
        request: Request<SetFirstDayOfWeekRequest>,
    ) -> Result<Response<()>, Status> {
        let weekday =
            map_weekday_from_proto(request.into_inner().first_day_of_week()).ok_or_else(|| {
                Status::with_error_details(
                    tonic::Code::InvalidArgument,
                    GrpcError::BadRequest.to_string(),
                    ErrorDetails::with_bad_request_violation(
                        "first_day_of_week",
                        "value cannot be unspecified",
                    ),
                )
            })?;

        let mut config = self.config_handle.write().await;
        config.set_first_day_of_week(weekday);

        config.save().await.map_err(|e| {
            error!("Failed to save first_day_of_week, error {e}");
            Status::internal("Failed to save first_day_of_week")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn set_temperature_unit(
        &self,
        request: Request<SetTemperatureUnitRequest>,
    ) -> Result<Response<()>, Status> {
        let temperature_unit = map_temperature_unit_from_proto(
            request.into_inner().temperature_unit(),
        )
        .ok_or_else(|| {
            Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request_violation(
                    "temperature_unit",
                    "value cannot be unspecified",
                ),
            )
        })?;

        let mut config = self.config_handle.write().await;
        config.set_temperature_unit(temperature_unit);

        config.save().await.map_err(|e| {
            error!("Failed to save temperature_unit, error {e}");
            Status::internal("Failed to save temperature_unit")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn set_unit_system(
        &self,
        request: Request<SetUnitSystemRequest>,
    ) -> Result<Response<()>, Status> {
        let unit_system = map_unit_system_from_proto(request.into_inner().unit_system())
            .ok_or_else(|| {
                Status::with_error_details(
                    tonic::Code::InvalidArgument,
                    GrpcError::BadRequest.to_string(),
                    ErrorDetails::with_bad_request_violation(
                        "unit_system",
                        "value cannot be unspecified",
                    ),
                )
            })?;

        let mut config = self.config_handle.write().await;
        config.set_unit_system(unit_system);

        config.save().await.map_err(|e| {
            error!("Failed to save unit_system, error {e}");
            Status::internal("Failed to save unit_system")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn show_seconds_in_status_bar(
        &self,
        request: Request<bool>,
    ) -> Result<Response<()>, Status> {
        let mut config = self.config_handle.write().await;
        config.show_seconds_in_status_bar(request.into_inner());

        config.save().await.map_err(|e| {
            error!("Failed to show/hide seconds in status bar, error {e}");
            Status::internal("Failed to show/hide seconds in status bar")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn get_led_settings(
        &self,
        _request: Request<()>,
    ) -> Result<Response<LedSettingsResponse>, Status> {
        let led_settings = self.system_manager.led_settings().await;

        Ok(Response::new(LedSettingsResponse {
            led_enabled: led_settings.led_enabled,
            led_enabled_nightmode: led_settings.led_enabled_night_mode,
        }))
    }

    async fn set_led_enabled(&self, request: Request<bool>) -> Result<Response<()>, Status> {
        let led_enabled = request.into_inner();

        self.system_manager
            .set_led_enabled(led_enabled)
            .await
            .map_err(|e| {
                error!("Failed to set led_enabled, error {e}");
                Status::internal("Failed to save led_enabled")
            })?;

        Ok(tonic::Response::new(()))
    }

    async fn set_led_enabled_nightmode(
        &self,
        request: Request<bool>,
    ) -> Result<Response<()>, Status> {
        let enabled = request.into_inner();

        self.system_manager
            .set_led_enabled_night_mode(enabled)
            .await
            .map_err(|e| {
                error!("Failed to set led_enabled_nightmode, error {e}");
                Status::internal("Failed to save led_enabled_nightmode")
            })?;

        Ok(tonic::Response::new(()))
    }

    async fn get_boot_sound_settings(
        &self,
        _request: Request<()>,
    ) -> Result<Response<BootSoundSettingsResponse>, Status> {
        let boot_sound_enabled = self.config_handle.read().await.boot_sound_enabled();

        Ok(Response::new(BootSoundSettingsResponse {
            boot_sound_enabled,
        }))
    }

    async fn set_boot_sound_enabled(&self, request: Request<bool>) -> Result<Response<()>, Status> {
        let mut config = self.config_handle.write().await;
        let enabled = request.into_inner();

        if config.boot_sound_enabled() == enabled {
            return Ok(tonic::Response::new(()));
        }

        config.set_boot_sound_enabled(enabled);

        config.save().await.map_err(|e| {
            error!("Failed to save boot_sound_enabled, error {e}");
            Status::internal("Failed to save boot_sound_enabled")
        })?;

        Ok(tonic::Response::new(()))
    }

    async fn set_nightmode_screen_off_timeout(
        &self,
        request: Request<u32>,
    ) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        let timeout = if value == 0 { None } else { Some(value) };

        self.system_manager
            .set_night_mode_screen_off_timeout(timeout)
            .await
            .map_err(|e| {
                error!("Failed to set night mode screen off timeout, error: {e}");
                Status::internal("Failed to set night mode screen off timeout")
            })?;

        Ok(Response::new(()))
    }
}

fn clamp_brightness(value: u32) -> u32 {
    value.clamp(API_BRIGHTNESS_MIN, API_BRIGHTNESS_MAX)
}

fn validate_sound_volume(value: u32) -> Result<(), Status> {
    if value > API_SOUND_VOLUME_MAX {
        return Err(Status::invalid_argument(format!(
            "Invalid sound volume. Value must be within a range [{API_SOUND_VOLUME_MIN}-{API_SOUND_VOLUME_MAX}]"
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

fn map_temperature_unit_from_proto(value: web::TemperatureUnit) -> Option<TemperatureUnit> {
    match value {
        web::TemperatureUnit::Unspecified => None,
        web::TemperatureUnit::Celsius => Some(TemperatureUnit::Celsius),
        web::TemperatureUnit::Fahrenheit => Some(TemperatureUnit::Fahrenheit),
    }
}

fn map_unit_system_to_proto(value: &UnitSystem) -> web::UnitSystem {
    match value {
        UnitSystem::Metric => web::UnitSystem::Metric,
        UnitSystem::Imperial => web::UnitSystem::Imperial,
    }
}

fn map_unit_system_from_proto(value: web::UnitSystem) -> Option<UnitSystem> {
    match value {
        web::UnitSystem::Unspecified => None,
        web::UnitSystem::Metric => Some(UnitSystem::Metric),
        web::UnitSystem::Imperial => Some(UnitSystem::Imperial),
    }
}
